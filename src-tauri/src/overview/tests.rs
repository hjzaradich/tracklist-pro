//! Overview number tests. Everything is made up: a small library with
//! known answers in a migrated database in a temp dir, and a generated one
//! of 100,000 tracks.

use std::cell::Cell;
use std::path::Path;

use rusqlite::Connection;
use serde_json::{json, Value};

use super::*;
use crate::db::Writer;
use crate::paths::Volumes;
use crate::volume::{Volume, VolumeId};

/// No drive is looked up: the overview never needs one for these libraries.
struct NoVolumes;

impl Volumes for NoVolumes {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up",
            path.display()
        )))
    }

    fn mount_path(&self, _: &VolumeId) -> Option<std::path::PathBuf> {
        None
    }
}

/// "Now" in every test: 2026-10-10 12:00 UTC.
const NOW_MS: i64 = 1_791_633_600_000;
const HOUR_MS: i64 = 3_600_000;

fn open(dir: &Path) -> Writer {
    Writer::open(&crate::write_guard::test_path(dir, crate::db::DB_FILE_NAME)).unwrap()
}

fn text(key: &str, text: &str) -> Value {
    json!({"key": key, "value": {"type": "text", "text": text}})
}

/// A library database with one volume and one music folder (id 1).
struct Lib {
    writer: Writer,
    next_track_id: Cell<i64>,
    // Last: fields drop in order, so the connections close before the folder goes.
    _dir: tempfile::TempDir,
}

impl Lib {
    fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = open(dir.path());
        writer
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES ('serial=1A2B3C4D', 'internal', 'C:\\');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (1, 'Music', 'Music');",
                )
            })
            .unwrap();
        Lib {
            _dir: dir,
            writer,
            next_track_id: Cell::new(1),
        }
    }

    fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> T {
        self.writer.call(f).unwrap()
    }

    fn exec(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) -> i64 {
        self.run(move |c| {
            c.execute(sql, params)?;
            Ok(c.last_insert_rowid())
        })
    }

    fn overview(&self) -> Overview {
        self.run(|c| overview_at(c, &NoVolumes, NOW_MS))
    }

    /// The stored form of the moment `ms` milliseconds before [`NOW_MS`].
    fn ms_ago(&self, ms: i64) -> String {
        self.run(move |c| {
            c.query_row(
                "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1 / 1000.0, 'unixepoch')",
                [NOW_MS - ms],
                |r| r.get(0),
            )
        })
    }

    /// The stored form of the moment `hours` before [`NOW_MS`].
    fn ago(&self, hours: i64) -> String {
        self.ms_ago(hours * HOUR_MS)
    }

    /// A track (a `recording`).
    /// The name is only for reading the test: nothing fills
    /// `recording.title` today, so a track is shown by its tags or its
    /// file's name.
    fn track(&self, _name: &str) -> i64 {
        self.exec("INSERT INTO recording DEFAULT VALUES", [])
    }

    /// A file of `track` that is its best, `ms` long, on the Music folder.
    fn file(&self, track: i64, name: &str, ms: Option<i64>, tags: Option<Value>) -> i64 {
        let file = self.exec(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, duration_ms, raw_tags)
             VALUES (1, ?1, ?1, ?2, ?3)",
            (name.to_owned(), ms, tags.map(|t| t.to_string())),
        );
        self.exec(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'best')",
            (track, file),
        );
        file
    }

    /// Another file of `track`, not its best.
    fn other_file(&self, track: i64, name: &str, tags: Option<Value>) -> i64 {
        let file = self.exec(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, raw_tags)
             VALUES (1, ?1, ?1, ?2)",
            (name.to_owned(), tags.map(|t| t.to_string())),
        );
        self.exec(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'undecided')",
            (track, file),
        );
        file
    }

    /// Adds `track` to the Library, linked to `file`, `hours` ago.
    fn add(&self, track: i64, file: i64, hours: i64) -> i64 {
        let at = self.ago(hours);
        self.exec(
            "INSERT INTO library_track (recording_id, linked_file_id, added_at)
             VALUES (?1, ?2, ?3)",
            (track, file, at),
        )
    }

    /// A Library track of its own, with a file of the given length.
    fn library_track(&self, name: &str, ms: Option<i64>, tags: Option<Value>) -> (i64, i64) {
        let track = self.track(name);
        let file = self.file(track, &format!("{name}.mp3"), ms, tags);
        (track, self.add(track, file, 1))
    }

    /// A rekordbox entry for `file` (and its track), attributes as given.
    fn rekordbox(&self, file: i64, attributes: Value, probable: bool) {
        let id = self.next_track_id.get();
        self.next_track_id.set(id + 1);
        let mut attributes = attributes;
        attributes["TrackID"] = json!(id.to_string());
        attributes["Location"] = json!(format!("file://localhost/C:/Music/{id}.mp3"));
        self.exec(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, recording_id, relink_method,
                  relink_probable)
             VALUES (?1, ?2, '2026-09-30T10:00:00.000Z', ?3,
                     (SELECT recording_id FROM recording_file WHERE file_id = ?3), ?4, ?5)",
            (
                attributes.to_string(),
                format!("C:/Music/{id}.mp3"),
                file,
                if probable { "filename_only" } else { "path" },
                probable,
            ),
        );
    }

    /// Writes the `analysis` rows rekordbox gives, as the attach job does.
    fn attach(&self) {
        self.run(|c| crate::attach::attach(c).map(|_| ()));
    }

    fn job(&self, kind: &'static str, status: &'static str) {
        self.exec(
            "INSERT INTO job (kind, status, error, finished_at)
             VALUES (?1, ?2, CASE ?2 WHEN 'failed' THEN 'it failed' END,
                     CASE WHEN ?2 IN ('done', 'failed', 'cancelled')
                          THEN '2026-10-10T11:00:00.000Z' END)",
            (kind, status),
        );
    }

    fn crate_with(&self, name: &'static str, kind: &'static str) -> i64 {
        self.exec(
            "INSERT INTO crate (kind, name, rules) VALUES (?1, ?2, CASE ?1 WHEN 'smart' THEN '{}' END)",
            (kind, name),
        )
    }

    fn put_in_crate(&self, crate_id: i64, library_track: i64, kind: &'static str) {
        self.exec(
            "INSERT INTO crate_entry (crate_id, library_track_id, kind) VALUES (?1, ?2, ?3)",
            (crate_id, library_track, kind),
        );
    }
}

// The Library's size.

#[test]
fn the_library_counts_its_tracks_and_adds_up_their_lengths() {
    let lib = Lib::new();
    lib.library_track("Alpha", Some(180_000), None);
    lib.library_track("Beta", Some(240_500), None);
    // A track whose length isn't read yet adds nothing.
    lib.library_track("Gamma", None, None);
    // A track in All music only is not in the Library.
    let other = lib.track("Delta");
    lib.file(other, "Delta.mp3", Some(1_000_000), None);

    let o = lib.overview();
    assert_eq!(o.tracks, 3);
    assert_eq!(o.total_length_ms, 420_500);
}

#[test]
fn a_track_whose_file_is_gone_or_on_an_unplugged_drive_still_counts_with_its_length() {
    let lib = Lib::new();
    let (_, gone) = lib.library_track("Alpha", Some(100_000), None);
    let (_, away) = lib.library_track("Beta", Some(200_000), None);
    // The file is gone from disk (the scan marks it).
    lib.exec(
        "UPDATE file SET present = 0 WHERE rel_path = 'Alpha.mp3'",
        [],
    );
    // The second file is on a drive that isn't plugged in: its volume has
    // no mount point, and the file stays present.
    lib.exec(
        "INSERT INTO volume (identity, kind) VALUES ('serial=0000FFFF', 'external')",
        [],
    );
    lib.exec(
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (2, 'Gig', 'Gig')",
        [],
    );
    lib.exec(
        "UPDATE file SET music_folder_id = 2 WHERE rel_path = 'Beta.mp3'",
        [],
    );
    let _ = (gone, away);

    let o = lib.overview();
    assert_eq!((o.tracks, o.total_length_ms), (2, 300_000));
}

#[test]
fn a_hidden_file_does_not_take_its_track_out_of_the_library() {
    let lib = Lib::new();
    lib.library_track("Alpha", Some(100_000), None);
    lib.exec("UPDATE file SET hidden_reason = 'user'", []);
    let o = lib.overview();
    assert_eq!((o.tracks, o.total_length_ms), (1, 100_000));
}

#[test]
fn a_library_track_with_no_file_link_counts_and_has_no_length() {
    let lib = Lib::new();
    let track = lib.track("Alpha");
    lib.exec(
        "INSERT INTO library_track (recording_id, linked_file_id, source_status)
         VALUES (?1, NULL, 'missing')",
        [track],
    );
    let o = lib.overview();
    assert_eq!((o.tracks, o.total_length_ms), (1, 0));
    assert_eq!(o.low_quality, 0);
    assert!(!o.counting.quality);
}

#[test]
fn an_empty_library_gives_zeroes_and_nothing_counting() {
    let o = Lib::new().overview();
    assert_eq!(o.tracks, 0);
    assert_eq!((o.in_crates, o.in_no_crate), (0, 0));
    assert_eq!((o.analyzed_by_rekordbox, o.not_analyzed), (0, 0));
    assert_eq!((o.low_quality, o.all_music), (0, 0));
    assert_eq!(o.last_sent_ms, None);
    assert!(o.genres.is_empty());
    assert_eq!(o.no_genre, 0);
    assert!(!o.rekordbox_read);
    assert_eq!(o.counting, Counting::default());
}

// Crates.

#[test]
fn a_track_in_two_crates_counts_once_and_a_smart_crates_rules_are_not_membership() {
    let lib = Lib::new();
    let (_, a) = lib.library_track("Alpha", None, None);
    let (_, b) = lib.library_track("Beta", None, None);
    let (_, c) = lib.library_track("Gamma", None, None);
    let warmup = lib.crate_with("Warmup", "static");
    let peak = lib.crate_with("Peak", "static");
    let smart = lib.crate_with("Fast", "smart");
    lib.put_in_crate(warmup, a, "member");
    lib.put_in_crate(peak, a, "member");
    lib.put_in_crate(peak, b, "member");
    lib.put_in_crate(smart, c, "always_include");

    let o = lib.overview();
    assert_eq!((o.tracks, o.in_crates, o.in_no_crate), (3, 2, 1));
}

// What rekordbox analyzed.

#[test]
fn a_track_is_analyzed_when_rekordbox_gave_a_bpm_or_a_key() {
    let lib = Lib::new();
    let (_, bpm_only) = lib.library_track("Has a BPM", None, None);
    let (_, key_only) = lib.library_track("Has a key", None, None);
    let (_, both) = lib.library_track("Has both", None, None);
    let (_, neither) = lib.library_track("Has neither", None, None);
    let (_, probable) = lib.library_track("Probably this one", None, None);
    lib.library_track("Unknown to rekordbox", None, None);
    let file = |name: &str| -> i64 {
        lib.run({
            let name = format!("{name}.mp3");
            move |c| {
                c.query_row("SELECT id FROM file WHERE rel_path = ?1", [name], |r| {
                    r.get(0)
                })
            }
        })
    };
    let _ = (bpm_only, key_only, both, neither, probable);
    lib.rekordbox(file("Has a BPM"), json!({"AverageBpm": "128.00"}), false);
    lib.rekordbox(file("Has a key"), json!({"Tonality": "Am"}), false);
    lib.rekordbox(
        file("Has both"),
        json!({"AverageBpm": "126.50", "Tonality": "8A"}),
        false,
    );
    // rekordbox knows the track but never analyzed it: BPM 0, no key.
    lib.rekordbox(file("Has neither"), json!({"AverageBpm": "0.00"}), false);
    // A match that isn't trusted gives nothing.
    lib.rekordbox(
        file("Probably this one"),
        json!({"AverageBpm": "140.00", "Tonality": "5A"}),
        true,
    );
    lib.attach();

    let o = lib.overview();
    assert_eq!(o.tracks, 6);
    assert_eq!((o.analyzed_by_rekordbox, o.not_analyzed), (3, 3));
    assert!(o.rekordbox_read);
}

#[test]
fn without_a_rekordbox_read_every_track_is_not_analyzed_and_the_overview_says_so() {
    let lib = Lib::new();
    lib.library_track("Alpha", None, None);
    let o = lib.overview();
    assert_eq!((o.analyzed_by_rekordbox, o.not_analyzed), (0, 1));
    assert!(!o.rekordbox_read);
}

#[test]
fn a_bpm_or_key_from_another_source_is_not_rekordbox_analyzing_the_track() {
    let lib = Lib::new();
    for source in ["tag", "mik", "local"] {
        let (track, _) = lib.library_track(source, None, None);
        lib.exec(
            "INSERT INTO analysis (recording_id, source, bpm, key) VALUES (?1, ?2, 128.0, '8A')",
            (track, source),
        );
    }
    let (rekordbox, _) = lib.library_track("From rekordbox", None, None);
    lib.exec(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (?1, 'rekordbox', 128.0)",
        [rekordbox],
    );
    let o = lib.overview();
    assert_eq!((o.analyzed_by_rekordbox, o.not_analyzed), (1, 3));
}

#[test]
fn a_track_with_two_rekordbox_entries_takes_the_one_attach_picks() {
    let lib = Lib::new();
    // The entry on the track's best file wins over one on another file,
    // however often the other was played; the pick has no BPM, so the track
    // is not analyzed even though the other entry has one.
    let two_files = lib.track("Two files");
    let best = lib.file(two_files, "Two files.mp3", None, None);
    let other = lib.other_file(two_files, "Two files other.mp3", None);
    lib.add(two_files, best, 1);
    lib.rekordbox(
        best,
        json!({"Genre": "On best file", "PlayCount": "1", "AverageBpm": "0.00"}),
        false,
    );
    lib.rekordbox(
        other,
        json!({"Genre": "On the other file", "PlayCount": "9", "AverageBpm": "128.00"}),
        false,
    );
    // On one file, the most played wins (the lower TrackID would lose).
    let (_, _) = lib.library_track("One file", None, None);
    let one = file_of(&lib, "One file");
    lib.rekordbox(
        one,
        json!({"Genre": "Played most", "PlayCount": "9", "AverageBpm": "120.00"}),
        false,
    );
    lib.rekordbox(
        one,
        json!({"Genre": "Played less", "PlayCount": "1", "AverageBpm": "0.00"}),
        false,
    );
    lib.attach();

    let o = lib.overview();
    let got: Vec<(&str, u32)> = o
        .genres
        .iter()
        .map(|g| (g.name.as_str(), g.tracks))
        .collect();
    assert_eq!(got, [("On best file", 1), ("Played most", 1)]);
    assert_eq!((o.analyzed_by_rekordbox, o.not_analyzed), (1, 1));
}

// Quality.

/// Makes the verdict of each file current, as the quality job does.
fn settle(lib: &Lib) {
    lib.run(|c| crate::quality::settle(c).map(|_| ()));
}

#[test]
fn a_track_is_low_quality_when_its_files_current_verdict_is_a_problem() {
    let lib = Lib::new();
    lib.library_track("Broken", None, None);
    lib.library_track("Cut off", None, None);
    lib.library_track("Thin", None, None);
    lib.library_track("Fine", None, None);
    lib.exec(
        "UPDATE file SET quality_verdict = 'broken' WHERE rel_path = 'Broken.mp3'",
        [],
    );
    lib.exec(
        "UPDATE file SET quality_verdict = 'truncated' WHERE rel_path = 'Cut off.mp3'",
        [],
    );
    lib.exec(
        "UPDATE file SET codec = 'mp3', bitrate = 64 WHERE rel_path = 'Thin.mp3'",
        [],
    );
    // A hidden or missing file is still the Library track's file.
    lib.exec(
        "UPDATE file SET present = 0 WHERE rel_path = 'Broken.mp3'",
        [],
    );
    settle(&lib);

    let o = lib.overview();
    assert_eq!(o.low_quality, 3);
    assert!(!o.counting.quality, "every verdict is current");
}

#[test]
fn a_stale_verdict_is_no_verdict_and_holds_the_quality_flag_only_while_a_quality_job_is_there() {
    let flagged = || {
        let lib = Lib::new();
        lib.library_track("Broken", None, None);
        lib.exec("UPDATE file SET quality_verdict = 'broken'", []);
        lib
    };

    // No verdict row yet and no job: not counted (and not "ok" either), and
    // nothing is counting, so the screen never waits for ever.
    let lib = flagged();
    let o = lib.overview();
    assert_eq!(o.low_quality, 0);
    assert!(!o.counting.quality);

    // A failed or a cancelled job settled nothing and holds nothing.
    for status in ["failed", "cancelled", "done"] {
        let lib = flagged();
        lib.job("quality", status);
        let o = lib.overview();
        assert_eq!((o.low_quality, o.counting.quality), (0, false), "{status}");
    }

    // A queued or running job is counting.
    for status in ["queued", "running"] {
        let lib = flagged();
        lib.job("quality", status);
        let o = lib.overview();
        assert_eq!((o.low_quality, o.counting.quality), (0, true), "{status}");
    }

    // Settled, the verdict counts; a change makes it stale again.
    let lib = flagged();
    settle(&lib);
    let o = lib.overview();
    assert_eq!((o.low_quality, o.counting.quality), (1, false));
    lib.exec("UPDATE file SET bitrate = 320", []);
    let o = lib.overview();
    assert_eq!((o.low_quality, o.counting.quality), (0, false));
    settle(&lib);
    assert_eq!(lib.overview().low_quality, 1);
}

#[test]
fn a_file_outside_the_library_with_a_stale_verdict_holds_nothing() {
    let lib = Lib::new();
    let other = lib.track("Not mine");
    lib.file(other, "Not mine.mp3", None, None);
    assert!(!lib.overview().counting.quality);
}

// When it was last sent.

#[test]
fn last_sent_is_the_latest_send_on_a_track_a_removed_track_or_a_crate() {
    let lib = Lib::new();
    assert_eq!(lib.overview().last_sent_ms, None);

    let (track, library_track) = lib.library_track("Alpha", None, None);
    assert_eq!(lib.overview().last_sent_ms, None);

    lib.exec(
        "UPDATE library_track SET last_exported_at = '2026-09-30T10:00:00.123Z' WHERE id = ?1",
        [library_track],
    );
    assert_eq!(lib.overview().last_sent_ms, Some(1_790_762_400_123));

    // A removed track's send is a send too.
    let gone = lib.track("Gone");
    lib.exec(
        "INSERT INTO library_removal (recording_id, last_exported_at)
         VALUES (?1, '2026-10-01T08:30:00.000Z')",
        [gone],
    );
    assert_eq!(lib.overview().last_sent_ms, Some(1_790_843_400_000));
    let _ = track;
}

// All music.

#[test]
fn all_music_counts_each_track_with_a_visible_file_once() {
    let lib = Lib::new();
    // In the Library, and not.
    lib.library_track("Alpha", None, None);
    let beta = lib.track("Beta");
    lib.file(beta, "Beta.mp3", None, None);
    // Two files, one track.
    let gamma = lib.track("Gamma");
    lib.file(gamma, "Gamma.mp3", None, None);
    let extra = lib.exec(
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'Gamma 2.mp3', 'Gamma 2.mp3')",
        [],
    );
    lib.exec(
        "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'extra')",
        (gamma, extra),
    );
    // A track with no file isn't in All music.
    lib.track("No file");
    // Only hidden files: not shown.
    let hidden = lib.track("Hidden");
    let hidden_file = lib.file(hidden, "Hidden.mp3", None, None);
    lib.exec(
        "UPDATE file SET hidden_reason = 'user' WHERE id = ?1",
        [hidden_file],
    );
    // One hidden file and one shown: shown.
    let mixed = lib.track("Mixed");
    let mixed_file = lib.file(mixed, "Mixed.mp3", None, None);
    lib.exec(
        "UPDATE file SET hidden_reason = 'inbox_cleared' WHERE id = ?1",
        [mixed_file],
    );
    let shown = lib.exec(
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'Mixed 2.mp3', 'Mixed 2.mp3')",
        [],
    );
    lib.exec(
        "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'extra')",
        (mixed, shown),
    );

    assert_eq!(lib.overview().all_music, 4);
}

#[test]
fn outside_the_library_counts_the_tracks_in_all_music_that_are_not_library_tracks() {
    let lib = Lib::new();
    // In the Library: in All music, not outside.
    lib.library_track("Alpha", None, None);
    // Not in the Library, with a file shown: outside.
    let beta = lib.track("Beta");
    let beta_file = lib.file(beta, "Beta.mp3", None, None);
    // Two files, one track: counted once.
    let gamma = lib.track("Gamma");
    lib.file(gamma, "Gamma.mp3", None, None);
    lib.other_file(gamma, "Gamma 2.mp3", None);
    // A track with no file, and one with only a hidden file: not in All music.
    lib.track("No file");
    let hidden = lib.track("Hidden");
    let hidden_file = lib.file(hidden, "Hidden.mp3", None, None);
    lib.exec(
        "UPDATE file SET hidden_reason = 'user' WHERE id = ?1",
        [hidden_file],
    );
    // A Library track whose only file is hidden is in neither count.
    lib.library_track("Library hidden", None, None);
    lib.exec(
        "UPDATE file SET hidden_reason = 'user' WHERE rel_path = 'Library hidden.mp3'",
        [],
    );

    let o = lib.overview();
    assert_eq!((o.all_music, o.outside_library), (3, 2));

    // Adding Beta to the Library takes it out of "outside".
    lib.add(beta, beta_file, 1);
    let o = lib.overview();
    assert_eq!((o.all_music, o.outside_library), (3, 1));
}

// Energy.

/// An `analysis` row that holds only an energy.
fn energy_row(lib: &Lib, track: i64, source: &'static str, energy: i64) {
    lib.exec(
        "INSERT INTO analysis (recording_id, source, energy) VALUES (?1, ?2, ?3)",
        (track, source, energy),
    );
}

#[test]
fn energy_counts_the_level_each_library_track_shows_and_the_tracks_with_none() {
    let lib = Lib::new();
    // The user's energy beats Mixed In Key's, the local estimate's and the
    // audio model's.
    let (a, _) = lib.library_track("User wins", None, None);
    energy_row(&lib, a, "user", 8);
    energy_row(&lib, a, "mik", 5);
    energy_row(&lib, a, "local", 3);
    energy_row(&lib, a, "ml", 2);
    // Mixed In Key's beats the local estimate's.
    let (b, _) = lib.library_track("Mixed In Key wins", None, None);
    energy_row(&lib, b, "mik", 5);
    energy_row(&lib, b, "local", 3);
    // The local estimate's beats the audio model's.
    let (c, _) = lib.library_track("Local wins", None, None);
    energy_row(&lib, c, "local", 3);
    energy_row(&lib, c, "ml", 9);
    // The audio model's counts when it is the only one.
    let (d, _) = lib.library_track("Model only", None, None);
    energy_row(&lib, d, "ml", 9);
    // The top level and the bottom level are both counted.
    let (e, _) = lib.library_track("Top", None, None);
    energy_row(&lib, e, "user", 10);
    let (f, _) = lib.library_track("Bottom", None, None);
    energy_row(&lib, f, "user", 1);
    // Two tracks at one level.
    let (g, _) = lib.library_track("Also eight", None, None);
    energy_row(&lib, g, "user", 8);
    // A BPM or a key from rekordbox is no energy; nor is no analysis at all.
    let (h, _) = lib.library_track("Rekordbox BPM only", None, None);
    lib.exec(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (?1, 'rekordbox', 128.0)",
        [h],
    );
    lib.library_track("No analysis", None, None);
    // A track outside the Library is not counted, whatever its energy.
    let outside = lib.track("Outside");
    lib.file(outside, "Outside.mp3", None, None);
    energy_row(&lib, outside, "user", 4);

    let o = lib.overview();
    assert_eq!(o.energy, [1, 0, 1, 0, 1, 0, 0, 2, 1, 1]);
    assert_eq!(o.no_energy, 2);
    assert_eq!(
        o.energy.iter().sum::<u32>() + o.no_energy,
        o.tracks,
        "every Library track is in exactly one bar"
    );
}

#[test]
fn energy_is_ten_empty_levels_and_no_energy_when_nothing_has_any() {
    let lib = Lib::new();
    let o = lib.overview();
    assert_eq!((o.energy.clone(), o.no_energy), (vec![0; 10], 0));

    lib.library_track("Alpha", None, None);
    lib.library_track("Beta", None, None);
    let o = lib.overview();
    assert_eq!((o.energy, o.no_energy), (vec![0; 10], 2));
}

// This week.

#[test]
fn this_week_is_the_last_seven_days_and_its_edges_are_exact() {
    let lib = Lib::new();
    let track = |name: &str, hours: i64| {
        let t = lib.track(name);
        let f = lib.file(t, &format!("{name}.mp3"), None, None);
        lib.add(t, f, hours);
    };
    track("Just now", 0);
    track("Almost a week", 7 * 24 - 1);
    track("Exactly a week", 7 * 24);
    track("Over a week", 7 * 24 + 1);
    // The operation log: the same edges.
    for (hours, kind) in [(1, "add_to_library"), (7 * 24 + 1, "create_crate")] {
        let at = lib.ago(hours);
        lib.exec(
            "INSERT INTO operation (kind, created_at) VALUES (?1, ?2)",
            (kind, at),
        );
    }
    let o = lib.overview();
    assert_eq!(o.added_this_week, 3);
    assert_eq!(o.decisions_this_week, 1);
}

#[test]
fn decisions_count_the_operations_in_the_undo_log_and_an_undone_one_does_not_count() {
    let lib = Lib::new();
    for undone in [false, false, true] {
        let at = lib.ago(2);
        lib.exec(
            "INSERT INTO operation (kind, created_at, undone_at)
             VALUES ('add_to_library', ?1, CASE ?2 WHEN 1 THEN ?1 END)",
            (at, undone),
        );
    }
    // Three operations in the log, one of them undone (owner decision,
    // 2026-10-12): two decisions.
    assert_eq!(lib.overview().decisions_this_week, 2);
}

#[test]
fn the_decisions_week_reaches_back_exactly_seven_days() {
    let lib = Lib::new();
    let week_ms = 7 * 24 * HOUR_MS;
    for (kind, ms) in [
        ("just inside", week_ms - 1),
        ("exactly seven days", week_ms),
        ("just outside", week_ms + 1),
        ("a day outside", week_ms + 24 * HOUR_MS),
    ] {
        let at = lib.ms_ago(ms);
        lib.exec(
            "INSERT INTO operation (kind, created_at) VALUES (?1, ?2)",
            (kind, at),
        );
    }
    assert_eq!(lib.overview().decisions_this_week, 2);
}

#[test]
fn the_owners_answers_are_the_values_of_the_three_choices() {
    // ROADMAP 1.13, "What the numbers mean" (2026-10-12).
    assert_eq!(
        [COUNT_UNDONE_OPERATIONS, BLANK_GENRE_FALLS_BACK],
        [false, false]
    );
    assert_eq!(THIS_WEEK, "-7 days");
}

// Genres.

#[test]
fn a_track_rekordbox_knows_has_rekordboxs_genre_and_others_have_the_files() {
    let lib = Lib::new();
    let tagged = |genre: &str| Some(json!({"id3v2": [text("TCON", genre)]}));
    let (_, known) = lib.library_track("Known", None, tagged("Tag House"));
    let (_, unknown) = lib.library_track("Unknown", None, tagged("Techno"));
    let (_, probable) = lib.library_track("Probable", None, tagged("Disco"));
    let (_, blank) = lib.library_track("Blank in rekordbox", None, tagged("Trance"));
    lib.library_track("No tag", None, None);
    lib.library_track("Empty tag", None, Some(json!({})));
    let _ = (known, unknown, probable, blank);
    let file = |name: &str| -> i64 {
        lib.run({
            let name = format!("{name}.mp3");
            move |c| {
                c.query_row("SELECT id FROM file WHERE rel_path = ?1", [name], |r| {
                    r.get(0)
                })
            }
        })
    };
    lib.rekordbox(file("Known"), json!({"Genre": "Rekordbox House"}), false);
    // Only a probable match: rekordbox doesn't know the track yet.
    lib.rekordbox(file("Probable"), json!({"Genre": "Wrong"}), true);
    lib.rekordbox(file("Blank in rekordbox"), json!({"Genre": "  "}), false);

    let o = lib.overview();
    // A track rekordbox knows with a blank genre has no genre: its file's
    // tag ("Trance") is not used (owner decision, 2026-10-12).
    let expected = [("Disco", 1), ("Rekordbox House", 1), ("Techno", 1)];
    let got: Vec<(&str, u32)> = o
        .genres
        .iter()
        .map(|g| (g.name.as_str(), g.tracks))
        .collect();
    assert_eq!(got, expected);
    assert_eq!(o.no_genre, 3);
}

#[test]
fn genres_are_sorted_by_tracks_then_name_and_capitals_do_not_make_another_genre() {
    let lib = Lib::new();
    for (i, genre) in [
        "house", "House", "House", "Techno", "Ambient", "Ambient", "Disco",
    ]
    .into_iter()
    .enumerate()
    {
        lib.library_track(
            &format!("T{i}"),
            None,
            Some(json!({"id3v2": [text("TCON", genre)]})),
        );
    }
    let o = lib.overview();
    let got: Vec<(&str, u32)> = o
        .genres
        .iter()
        .map(|g| (g.name.as_str(), g.tracks))
        .collect();
    // "House" is spelled that way by most of its tracks.
    assert_eq!(
        got,
        [("House", 3), ("Ambient", 2), ("Disco", 1), ("Techno", 1)]
    );
    assert_eq!(o.no_genre, 0);
}

/// The awkward tags: what each is expected to give is whatever the send's
/// own rules ([`crate::send_values::tag_values`]) say, so the two can't
/// drift apart.
fn awkward_tags() -> Vec<(&'static str, Option<Value>)> {
    let cut =
        json!({"key": "TCON", "value": {"type": "text", "text": "Cut off", "full_len": 9000}});
    vec![
        ("no tags", None),
        ("an empty tag object", Some(json!({}))),
        (
            "an empty string",
            Some(json!({"id3v2": [text("TCON", "")]})),
        ),
        ("only blanks", Some(json!({"id3v2": [text("TCON", "   ")]}))),
        ("only a NUL", Some(json!({"id3v2": [text("TCON", "\0")]}))),
        (
            "several genre frames",
            Some(json!({"id3v2": [text("TCON", "House"), text("TCON", "Techno")]})),
        ),
        (
            "an empty frame before a filled one",
            Some(json!({"id3v2": [text("TCON", ""), text("TCON", "Disco")]})),
        ),
        (
            "mixed case",
            Some(json!({"id3v2": [text("TCON", "hOuSe")]})),
        ),
        (
            "two values in one frame",
            Some(json!({"id3v2": [text("TCON", "House\0Techno")]})),
        ),
        (
            "an ID3 code in brackets",
            Some(json!({"id3v2": [text("TCON", "(17)")]})),
        ),
        (
            "a bare ID3 number",
            Some(json!({"id3v2": [text("TCON", "17")]})),
        ),
        (
            "an ID3 code with a refinement",
            Some(json!({"id3v2": [text("TCON", "(17)Hard")]})),
        ),
        (
            "an unknown ID3 number",
            Some(json!({"id3v2": [text("TCON", "(999)")]})),
        ),
        (
            "a literal bracket",
            Some(json!({"id3v2": [text("TCON", "((x)")]})),
        ),
        (
            "an ID3v1 number only",
            Some(json!({"id3v1": [text("genre", "17")]})),
        ),
        ("a cut-off text", Some(json!({"id3v2": [cut]}))),
        (
            "a Vorbis key in capitals",
            Some(json!({"vorbis_comments": [text("GENRE", "Drum & Bass")]})),
        ),
        (
            "a Vorbis key in lower case",
            Some(json!({"vorbis_comments": [text("genre", "Jungle")]})),
        ),
        (
            "a repeated Vorbis item",
            Some(json!({"vorbis_comments": [text("GENRE", "House"), text("Genre", "Techno")]})),
        ),
        (
            "a repeated Vorbis item with a copy",
            Some(json!({"vorbis_comments": [text("GENRE", "House"), text("genre", "House")]})),
        ),
        (
            "an APE item",
            Some(json!({"ape": [text("Genre", "Electro")]})),
        ),
        (
            "an MP4 atom",
            Some(json!({"mp4_ilst": [text("\u{a9}gen", "Ambient")]})),
        ),
        (
            "a RIFF INFO field",
            Some(json!({"riff_info": [text("IGNR", "Pop")]})),
        ),
        (
            "an AIFF text chunk",
            Some(json!({"aiff_text": [text("NAME", "Not a genre")]})),
        ),
        (
            "ID3v2 before Vorbis",
            Some(
                json!({"vorbis_comments": [text("GENRE", "Vorbis")], "id3v2": [text("TCON", "Id3")]}),
            ),
        ),
        (
            "a blank ID3v2 genre before a Vorbis one",
            Some(
                json!({"id3v2": [text("TCON", " ")], "vorbis_comments": [text("GENRE", "Vorbis")]}),
            ),
        ),
        (
            "a genre among other frames",
            Some(
                json!({"id3v2": [text("TIT2", "Title"), text("COMM::eng", "Note"),
                                  text("TCON", "Garage"), text("TXXX:GENRE", "Not this")]}),
            ),
        ),
        (
            "binary and unrecorded frames",
            Some(json!({"id3v2": [
                {"key": "APIC", "value": {"type": "binary", "len": 1234}},
                {"key": "TCON", "value": {"type": "unrecorded"}},
                text("TCON", "Funk")]})),
        ),
    ]
}

#[test]
fn the_genre_read_from_a_files_tag_in_sql_equals_the_sends_own_reading_of_it() {
    for (case, tags) in awkward_tags() {
        let lib = Lib::new();
        lib.library_track("Track", None, tags.clone());
        let expected = tags.as_ref().and_then(|t| {
            crate::send_values::tag_values(&t.to_string(), &["Genre"])
                .first()
                .map(|v| v.value.trim().to_owned())
                .filter(|g| !g.is_empty())
        });
        let o = lib.overview();
        let got: Vec<(String, u32)> = o
            .genres
            .iter()
            .map(|g| (g.name.clone(), g.tracks))
            .collect();
        match expected {
            Some(genre) => {
                assert_eq!(got, [(genre, 1)], "{case}");
                assert_eq!(o.no_genre, 0, "{case}");
            }
            None => {
                assert!(got.is_empty(), "{case}: {got:?}");
                assert_eq!(o.no_genre, 1, "{case}");
            }
        }
    }
}

#[test]
fn the_awkward_tags_are_not_all_empty() {
    // Guards the test above against passing because nothing gives a genre.
    let given = awkward_tags()
        .into_iter()
        .filter_map(|(_, tags)| tags)
        .filter(|t| !crate::send_values::tag_values(&t.to_string(), &["Genre"]).is_empty())
        .count();
    assert!(given >= 15, "{given}");
}

// Missing tags and missing art.

/// A tag block with a title, an artist and a genre, whichever are given.
fn id3(title: Option<&str>, artist: Option<&str>, genre: Option<&str>) -> Option<Value> {
    let mut frames = Vec::new();
    for (key, value) in [("TIT2", title), ("TPE1", artist), ("TCON", genre)] {
        if let Some(value) = value {
            frames.push(text(key, value));
        }
    }
    Some(json!({ "id3v2": frames }))
}

fn file_of(lib: &Lib, name: &str) -> i64 {
    let name = format!("{name}.mp3");
    lib.run(move |c| {
        c.query_row("SELECT id FROM file WHERE rel_path = ?1", [name], |r| {
            r.get(0)
        })
    })
}

#[test]
fn a_track_is_missing_tags_when_it_has_no_title_no_artist_or_no_genre() {
    let lib = Lib::new();
    lib.library_track("Complete", None, id3(Some("A"), Some("B"), Some("House")));
    lib.library_track("No genre", None, id3(Some("A"), Some("B"), None));
    lib.library_track("No artist", None, id3(Some("A"), None, Some("House")));
    lib.library_track(
        "Blank artist",
        None,
        id3(Some("A"), Some("  "), Some("House")),
    );
    // No title tag: the file's name doesn't count as a title.
    lib.library_track(
        "Only the file name",
        None,
        id3(None, Some("B"), Some("House")),
    );
    lib.library_track("Nothing tagged", None, None);

    let o = lib.overview();
    assert_eq!(o.tracks, 6);
    assert_eq!(o.missing_tags, 5, "all but Complete");
    assert_eq!(o.no_genre, 2, "No genre and Nothing tagged");
}

#[test]
fn a_title_is_the_files_title_tag_or_rekordboxs_name_never_the_files_name() {
    let lib = Lib::new();
    // A track rekordbox doesn't know: its Title tag, or nothing.
    lib.library_track(
        "Unknown with a title tag",
        None,
        id3(Some("T"), Some("B"), Some("House")),
    );
    lib.library_track(
        "Unknown without a title tag",
        None,
        id3(None, Some("B"), Some("House")),
    );
    // A track rekordbox knows: its Name there, whatever the tags say.
    lib.library_track(
        "Known with a name",
        None,
        id3(None, Some("B"), Some("House")),
    );
    lib.library_track(
        "Known with a blank name",
        None,
        id3(Some("T"), Some("B"), Some("House")),
    );
    // The track's own title counts, too.
    let (own, _) = lib.library_track("Own title", None, id3(None, Some("B"), Some("House")));
    lib.exec("UPDATE recording SET title = 'Mine' WHERE id = ?1", [own]);
    let entry = |name: &str| json!({"Name": name, "Artist": "B", "Genre": "House"});
    lib.rekordbox(file_of(&lib, "Known with a name"), entry("N"), false);
    lib.rekordbox(file_of(&lib, "Known with a blank name"), entry(" "), false);

    let o = lib.overview();
    assert_eq!(o.tracks, 5);
    assert_eq!(
        o.missing_tags, 2,
        "the unknown track with no title tag and the known one with a blank name"
    );
    assert_eq!(o.no_genre, 0);
}

#[test]
fn a_track_rekordbox_knows_is_judged_by_rekordboxs_title_artist_and_genre() {
    let lib = Lib::new();
    lib.library_track("Known and complete", None, None);
    lib.library_track(
        "Known without an artist",
        None,
        id3(Some("A"), Some("B"), Some("House")),
    );
    lib.rekordbox(
        file_of(&lib, "Known and complete"),
        json!({"Name": "N", "Artist": "B", "Genre": "House"}),
        false,
    );
    // rekordbox's blank artist stands: the file's tag is not looked at.
    lib.rekordbox(
        file_of(&lib, "Known without an artist"),
        json!({"Name": "N", "Artist": "", "Genre": "House"}),
        false,
    );
    let o = lib.overview();
    assert_eq!(o.missing_tags, 1);
}

#[test]
fn a_track_whose_linked_file_is_gone_is_judged_by_its_stored_tags() {
    let lib = Lib::new();
    lib.library_track(
        "Gone and tagged",
        None,
        id3(Some("A"), Some("B"), Some("House")),
    );
    lib.library_track("Gone and bare", None, None);
    lib.exec("UPDATE file SET present = 0", []);
    let o = lib.overview();
    assert_eq!(o.missing_tags, 1);
}

#[test]
fn a_track_with_no_file_at_all_is_missing_its_tags() {
    let lib = Lib::new();
    let track = lib.track("Alpha");
    lib.exec(
        "INSERT INTO library_track (recording_id, linked_file_id, source_status)
         VALUES (?1, NULL, 'missing')",
        [track],
    );
    let o = lib.overview();
    assert_eq!((o.missing_tags, o.no_genre, o.missing_art), (1, 1, 0));
}

fn binary(key: &str, len: i64) -> Value {
    json!({"key": key, "value": {"type": "binary", "len": len}})
}

#[test]
fn a_track_is_missing_art_when_its_linked_files_tags_hold_no_picture() {
    let lib = Lib::new();
    let with = |name: &str, tags: Value| {
        lib.library_track(name, None, Some(tags));
    };
    with(
        "ID3 cover",
        json!({"id3v2": [text("TIT2", "A"), binary("APIC", 50_000)]}),
    );
    with(
        "ID3 cover with a description",
        json!({"id3v2": [binary("APIC:Front", 50_000)]}),
    );
    with("MP4 cover", json!({"mp4_ilst": [binary("covr", 9_000)]}));
    with(
        "FLAC cover",
        json!({"vorbis_comments": [binary("METADATA_BLOCK_PICTURE", 9_000)]}),
    );
    with(
        "FLAC cover with a description",
        json!({"vorbis_comments": [binary("METADATA_BLOCK_PICTURE:Front cover", 9_000)]}),
    );
    with(
        "APE cover",
        json!({"ape": [binary("Cover Art (Front)", 9_000)]}),
    );
    with(
        "APE cover in lower case",
        json!({"ape": [binary("cover art (back)", 9_000)]}),
    );
    // Not covers: other binary frames, an empty picture, a picture named in text.
    with(
        "Cue data only",
        json!({"id3v2": [binary("GEOB:Serato Markers2", 5_000)]}),
    );
    with("Empty picture", json!({"id3v2": [binary("APIC", 0)]}));
    with(
        "Text called APIC",
        json!({"id3v2": [text("APIC", "not a picture")]}),
    );
    with("Untagged", json!({}));
    // Tags never read: unknown, not missing.
    lib.library_track("Not read yet", None, None);
    // No linked file: nothing to look at.
    let nothing = lib.track("No file");
    lib.exec(
        "INSERT INTO library_track (recording_id, linked_file_id, source_status)
         VALUES (?1, NULL, 'missing')",
        [nothing],
    );
    // A gone file's stored tags still say.
    lib.library_track(
        "Gone with a cover",
        None,
        Some(json!({"id3v2": [binary("APIC", 1)]})),
    );
    lib.library_track("Gone without", None, Some(json!({})));
    lib.exec(
        "UPDATE file SET present = 0 WHERE rel_path IN ('Gone with a cover.mp3', 'Gone without.mp3')",
        [],
    );

    let o = lib.overview();
    assert_eq!(
        o.missing_art, 5,
        "cue data, empty, text, untagged, gone without"
    );
}

#[test]
fn a_cover_in_a_real_id3_tag_is_found_by_the_art_rule() {
    use crate::tags::test_audio::{id3_frame, id3v2, mp3};
    // An APIC frame: UTF-8, MIME, picture type, description, picture data.
    let mut body = vec![3u8];
    body.extend_from_slice(b"image/jpeg\0");
    body.push(3);
    body.extend_from_slice(b"front\0");
    body.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4]);
    let mut bytes = id3v2(&[id3_frame(b"APIC", &body)], 0);
    bytes.extend(mp3());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cover.mp3");
    std::fs::write(&path, bytes).unwrap();
    let read = crate::tags::read(&path).unwrap();

    // The scan stores the blocks like this (`read::file::raw_tags_json`).
    let mut blocks = serde_json::Map::new();
    for tag in &read.raw_tags {
        let kind = serde_json::to_value(tag.tag_type).unwrap();
        blocks.insert(
            kind.as_str().unwrap().to_owned(),
            serde_json::to_value(&tag.items).unwrap(),
        );
    }
    let lib = Lib::new();
    lib.library_track("Real cover", None, Some(Value::Object(blocks)));
    lib.library_track("Real bare", None, Some(json!({})));
    assert_eq!(lib.overview().missing_art, 1);
}

/// A library of awkward tracks for the short way of counting missing tags:
/// each is settled by a different rule, and the counts of the short way and
/// of asking `shown` about every track must agree.
fn awkward_library() -> Lib {
    let lib = Lib::new();
    let vorbis = |artist: &str| {
        Some(json!({"vorbis_comments": [text("GENRE", "House"), text("ARTIST", artist)]}))
    };
    lib.library_track("Tagged", None, id3(Some("A"), Some("B"), Some("House")));
    lib.library_track("Vorbis artist", None, vorbis("B"));
    lib.library_track("Vorbis blank artist", None, vorbis("   "));
    lib.library_track(
        "Lower case key",
        None,
        Some(json!({"ape": [text("genre", "House"), text("artist", "B")]})),
    );
    lib.library_track(
        "Artist is only a NUL",
        None,
        Some(json!({"id3v2": [text("TPE1", "\0"), text("TCON", "House")]})),
    );
    lib.library_track("Artist cut off", None, Some(json!({"id3v2": [
        {"key": "TPE1", "value": {"type": "text", "text": "B", "full_len": 9000}}, text("TCON", "House")]})));
    lib.library_track(
        "ID3v1 artist",
        None,
        Some(json!({"id3v2": [text("TCON", "House")], "id3v1": [text("artist", "B")]})),
    );
    lib.library_track(
        "AIFF artist",
        None,
        Some(json!({"id3v2": [text("TCON", "House")], "aiff_text": [text("AUTH", "B")]})),
    );
    lib.library_track(
        "Known and artist",
        None,
        id3(Some("A"), None, Some("House")),
    );
    lib.library_track(
        "Known and no artist",
        None,
        id3(Some("A"), Some("B"), Some("House")),
    );
    lib.library_track(
        "Known, entry also has a tag",
        None,
        id3(Some("A"), Some("B"), None),
    );
    lib.library_track("Gone", None, id3(Some("A"), Some("B"), Some("House")));
    lib.library_track(
        "Gone and no artist",
        None,
        id3(Some("A"), None, Some("House")),
    );
    lib.library_track("Probable only", None, id3(None, Some("B"), Some("House")));
    let titled = lib.track("Titled recording");
    lib.exec(
        "UPDATE recording SET title = 'Own title', artist = ' ' WHERE id = ?1",
        [titled],
    );
    let f = lib.file(
        titled,
        "Titled recording.mp3",
        None,
        id3(None, None, Some("House")),
    );
    lib.add(titled, f, 1);
    // The artist is in another file of the track, not the linked one.
    let sibling = lib.track("Artist in a sibling file");
    let linked = lib.file(
        sibling,
        "Sibling linked.mp3",
        None,
        id3(Some("A"), None, Some("House")),
    );
    lib.other_file(
        sibling,
        "Sibling other.mp3",
        id3(Some("A"), Some("B"), Some("House")),
    );
    lib.add(sibling, linked, 1);
    // A sibling that is gone is not asked.
    let gone_sibling = lib.track("Artist in a gone sibling");
    let linked = lib.file(
        gone_sibling,
        "Gone sibling linked.mp3",
        None,
        id3(Some("A"), None, Some("House")),
    );
    let other = lib.other_file(
        gone_sibling,
        "Gone sibling other.mp3",
        id3(Some("A"), Some("B"), Some("House")),
    );
    lib.exec("UPDATE file SET present = 0 WHERE id = ?1", [other]);
    lib.add(gone_sibling, linked, 1);
    // The track's own artist and title win over rekordbox and the tags.
    let (own, _) = lib.library_track("Own artist", None, id3(None, None, Some("House")));
    lib.exec(
        "UPDATE recording SET title = 'Mine', artist = 'Me' WHERE id = ?1",
        [own],
    );
    lib.rekordbox(
        file_of(&lib, "Own artist"),
        json!({"Name": "", "Artist": "", "Genre": "House"}),
        false,
    );
    // The title is in another file of the track, not the linked one.
    let title_sibling = lib.track("Title in a sibling file");
    let linked = lib.file(
        title_sibling,
        "Title sibling linked.mp3",
        None,
        id3(None, Some("B"), Some("House")),
    );
    lib.other_file(
        title_sibling,
        "Title sibling other.mp3",
        id3(Some("A"), None, None),
    );
    lib.add(title_sibling, linked, 1);
    // A known track with a blank name, and a gone file with no title tag.
    lib.library_track(
        "Known blank name",
        None,
        id3(Some("A"), Some("B"), Some("House")),
    );
    lib.rekordbox(
        file_of(&lib, "Known blank name"),
        json!({"Name": " ", "Artist": "B", "Genre": "House"}),
        false,
    );
    lib.library_track(
        "Gone, no title tag",
        None,
        id3(None, Some("B"), Some("House")),
    );
    lib.exec(
        "UPDATE file SET present = 0 WHERE rel_path = 'Gone, no title tag.mp3'",
        [],
    );
    // No linked file.
    let none = lib.track("No link");
    lib.exec(
        "INSERT INTO library_track (recording_id, linked_file_id, source_status) VALUES (?1, NULL, 'missing')",
        [none],
    );

    lib.rekordbox(
        file_of(&lib, "Known and artist"),
        json!({"Name": "N", "Artist": "B", "Genre": "House"}),
        false,
    );
    lib.rekordbox(
        file_of(&lib, "Known and no artist"),
        json!({"Name": "N", "Genre": "House"}),
        false,
    );
    lib.rekordbox(
        file_of(&lib, "Known, entry also has a tag"),
        json!({"Artist": "B", "Genre": "House"}),
        false,
    );
    // A probable entry is not rekordbox knowing the file.
    lib.rekordbox(
        file_of(&lib, "Probable only"),
        json!({"Artist": "", "Genre": "Wrong"}),
        true,
    );
    // A gone file with an entry: rekordbox's, if it is at its location.
    lib.rekordbox(
        file_of(&lib, "Gone"),
        json!({"Artist": "", "Genre": "House"}),
        false,
    );
    lib.exec(
        "UPDATE file SET present = 0 WHERE rel_path IN ('Gone.mp3', 'Gone and no artist.mp3')",
        [],
    );
    lib
}

/// The ids the short way counts and the ids asking `shown_tags` about every
/// track counts, sorted.
fn both_ways(lib: &Lib) -> (Vec<i64>, Vec<i64>) {
    lib.run(|c| {
        let mut short = missing_tag_ids(c, &NoVolumes, false)?;
        let mut long = missing_tag_ids(c, &NoVolumes, true)?;
        short.sort_unstable();
        long.sort_unstable();
        Ok((short, long))
    })
}

#[test]
fn the_short_way_of_counting_missing_tags_agrees_with_asking_for_every_shown_value() {
    let lib = awkward_library();
    let (short, long) = both_ways(&lib);
    assert_eq!(short, long);
    // Not everything is missing, and not nothing.
    let tracks = lib.overview().tracks as usize;
    assert!(
        short.len() > 5 && short.len() < tracks,
        "{} of {tracks}",
        short.len()
    );
    assert_eq!(lib.overview().missing_tags as usize, short.len());
}

/// splitmix64: a seeded, platform-independent random source.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Nothing, a blank, or a value.
    fn text(&mut self) -> Option<&'static str> {
        [None, Some("  "), Some("\0"), Some("Value"), Some("Value")][self.below(5) as usize]
    }
}

/// A random block of tags in one of the formats, each of title, artist and
/// genre absent, blank or filled, keys in either case.
fn random_tags(rng: &mut Rng) -> Option<Value> {
    let (title, artist, genre) = (rng.text(), rng.text(), rng.text());
    let upper = rng.below(2) == 0;
    let pick = |upper: bool, a: &'static str, b: &'static str| if upper { a } else { b };
    let frames = |keys: [&'static str; 3]| -> Vec<Value> {
        [title, artist, genre]
            .into_iter()
            .zip(keys)
            .filter_map(|(value, key)| value.map(|v| text(key, v)))
            .collect()
    };
    match rng.below(6) {
        0 => None,
        1 => Some(json!({})),
        2 => Some(json!({"id3v2": frames(["TIT2", "TPE1", "TCON"])})),
        3 => Some(json!({"vorbis_comments": frames([
            pick(upper, "TITLE", "title"), pick(upper, "ARTIST", "artist"), pick(upper, "GENRE", "genre")])})),
        4 => Some(json!({"ape": frames(["Title", "Artist", "Genre"]),
                         "id3v1": frames(["title", "artist", "genre"])})),
        _ => Some(json!({"riff_info": frames(["INAM", "IART", "IGNR"])})),
    }
}

/// `count` random Library tracks: some without a linked file, some with a
/// gone one, some with a second file of the track (on disk or gone), the
/// track's own title and artist set now and then, and up to two rekordbox
/// entries (some only probable) with names, artists and genres absent,
/// blank or filled.
fn random_library(seed: u64, count: usize) -> Lib {
    let lib = Lib::new();
    let mut rng = Rng(seed);
    for n in 0..count {
        let track = lib.track("Random");
        let own = |rng: &mut Rng| match rng.below(4) {
            0 => Some("Own".to_owned()),
            1 => Some(" ".to_owned()),
            _ => None,
        };
        lib.exec(
            "UPDATE recording SET title = ?1, artist = ?2 WHERE id = ?3",
            (own(&mut rng), own(&mut rng), track),
        );
        let mut files = Vec::new();
        if rng.below(8) != 0 {
            let tags = random_tags(&mut rng);
            files.push(lib.file(track, &format!("Random {n}.mp3"), None, tags));
            if rng.below(3) == 0 {
                let tags = random_tags(&mut rng);
                files.push(lib.other_file(track, &format!("Random {n} other.mp3"), tags));
            }
            for file in &files {
                if rng.below(5) == 0 {
                    lib.exec("UPDATE file SET present = 0 WHERE id = ?1", [*file]);
                }
            }
        }
        match files.first() {
            Some(linked) => {
                lib.add(track, *linked, 1);
            }
            None => {
                lib.exec(
                    "INSERT INTO library_track (recording_id, linked_file_id, source_status)
                     VALUES (?1, NULL, 'missing')",
                    [track],
                );
            }
        }
        for _ in 0..rng.below(3) {
            if files.is_empty() {
                break;
            }
            let file = files[rng.below(files.len() as u64) as usize];
            let mut attributes = serde_json::Map::new();
            for key in ["Name", "Artist", "Genre"] {
                if let Some(value) = rng.text() {
                    attributes.insert(key.to_owned(), json!(value));
                }
            }
            attributes.insert("PlayCount".to_owned(), json!(rng.below(3).to_string()));
            let probable = rng.below(4) == 0;
            lib.rekordbox(file, Value::Object(attributes), probable);
        }
    }
    lib
}

#[test]
fn the_short_way_agrees_with_asking_for_every_track_on_four_hundred_random_ones() {
    // A fixed seed: the same four hundred tracks on every run.
    let lib = random_library(0x2026_1012, 400);
    let (short, long) = both_ways(&lib);
    assert_eq!(short, long);
    let tracks = lib.overview().tracks as usize;
    assert!(
        short.len() > tracks / 4 && short.len() < tracks,
        "{} of {tracks}",
        short.len()
    );
}

// Still counting.

#[test]
fn with_no_jobs_nothing_is_counting_whatever_is_stale() {
    let lib = Lib::new();
    lib.library_track("Alpha", None, None);
    assert_eq!(lib.overview().counting, Counting::default());
    settle(&lib);
    assert_eq!(lib.overview().counting, Counting::default());
}

#[test]
fn a_queued_or_running_job_holds_exactly_its_flag_and_a_finished_failed_or_cancelled_one_holds_none(
) {
    let flags = |files, rekordbox, quality| Counting {
        files,
        rekordbox,
        quality,
    };
    for (kind, held) in [
        ("scan", flags(true, false, false)),
        ("read", flags(true, false, false)),
        ("group", flags(true, false, false)),
        ("read_rekordbox", flags(false, true, false)),
        ("relink", flags(false, true, false)),
        ("attach", flags(false, true, false)),
        ("quality", flags(false, false, true)),
    ] {
        for status in ["queued", "running"] {
            let lib = Lib::new();
            lib.job(kind, status);
            assert_eq!(lib.overview().counting, held, "{kind} {status}");
        }
        let lib = Lib::new();
        for status in ["done", "failed", "cancelled"] {
            lib.job(kind, status);
        }
        assert_eq!(lib.overview().counting, Counting::default(), "{kind} ended");
    }
    // Several at once hold all their flags.
    let lib = Lib::new();
    lib.job("scan", "running");
    lib.job("attach", "queued");
    lib.job("quality", "queued");
    assert_eq!(lib.overview().counting, flags(true, true, true));
}

#[test]
fn work_that_changes_none_of_the_numbers_holds_no_flag() {
    let lib = Lib::new();
    lib.job("export", "running");
    lib.job("embed", "queued");
    assert_eq!(lib.overview().counting, Counting::default());
}

// Cost.

/// The tables a statement's plan scans end to end (`SCAN x` lines), and
/// whether every other table is reached through an index.
fn scans(conn: &Connection, sql: &str, args: impl rusqlite::Params) -> Vec<String> {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let rows = stmt
        .query_map(args, |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows.into_iter()
        .filter_map(|d| d.strip_prefix("SCAN ").map(str::to_owned))
        .map(|d| d.split_whitespace().next().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn no_statement_scans_a_table_it_does_not_have_to() {
    let lib = Lib::new();
    lib.run(|c| {
        // Tables the statements scan, and the table-valued functions that
        // open one file's tags (`blk`, `i`), which are not tables.
        let library = scans(c, &library_sql(), ["x"])
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(
            library,
            ["lt"],
            "the Library's numbers: one pass over its tracks"
        );
        let all_music = scans(c, ALL_MUSIC, []);
        assert_eq!(all_music, ["r"], "All music: one pass over the tracks");
        let mut energy = scans(c, ENERGY, []);
        energy.retain(|t| t != "s");
        assert_eq!(energy, ["lt"], "energy: one pass over the Library's tracks");
        let decisions = scans(c, &decisions_sql(), [0]);
        assert_eq!(decisions, ["operation"]);
        let mut genres = scans(c, &genres_sql(), []);
        genres.retain(|t| t != "blk" && t != "i" && t != "b");
        assert_eq!(genres, ["lt"], "genres: one pass over the Library's tracks");
        let mut per_track = scans(c, &missing_tags_sql(), []);
        per_track.retain(|t| t != "blk" && t != "i" && t != "b");
        assert_eq!(
            per_track,
            ["lt"],
            "missing tags: one pass over the Library's tracks"
        );
        let mut art = scans(c, MISSING_ART, []);
        art.retain(|t| t != "blk" && t != "i");
        assert_eq!(
            art,
            ["lt"],
            "missing art: one pass over the Library's tracks"
        );
        Ok(())
    });
}

#[test]
fn a_library_of_2000_tracks_is_read_in_one_pass_for_energy_and_outside_the_library() {
    // The plan, not a time: the statements this adds scan only the table
    // they count, and reach the others through an index, however many
    // tracks there are.
    let lib = Lib::new();
    build(&lib, 2_000);
    lib.run(|c| {
        let mut energy = scans(c, ENERGY, []);
        energy.retain(|t| t != "s");
        assert_eq!(energy, ["lt"], "energy: one pass over the Library's tracks");
        assert_eq!(
            scans(c, ALL_MUSIC, []),
            ["r"],
            "outside the Library: the pass All music makes"
        );
        Ok(())
    });
    let o = lib.overview();
    assert_eq!(o.energy.iter().sum::<u32>() + o.no_energy, 2_000);
    assert_eq!((o.all_music, o.outside_library), (2_000, 0));
}

/// A deterministic spread of tags and rekordbox data, by track number `n`.
fn made_up_genre(n: i64) -> Option<String> {
    if n % 3 == 0 {
        Some(format!("RB {}", n % 10))
    } else if n % 5 != 0 {
        Some(format!("Tag {}", n % 40))
    } else {
        None
    }
}

/// Builds a Library of `count` tracks by SQL alone: 3 in 3 are known to
/// rekordbox with a BPM, 2 in 5 are in a crate, 4 in 5 of the others have a
/// genre tag, 1 in 4 has an energy from the user and 1 in 6 one from Mixed In Key.
fn build(lib: &Lib, count: i64) {
    lib.run(move |c| {
        c.execute_batch(
            "INSERT INTO crate (id, kind, name) VALUES (1, 'static', 'Warmup');",
        )?;
        let tx = c.transaction()?;
        tx.execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
             INSERT INTO recording (id, title) SELECT i, 'Track ' || i FROM n;
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
             INSERT INTO file (id, music_folder_id, rel_path, rel_path_key, duration_ms, raw_tags)
             SELECT n.i, 1, 'f' || n.i || '.mp3', 'f' || n.i || '.mp3', 1000 + (n.i % 7) * 1000,
                    json_object('id3v2', (SELECT json_group_array(json(x)) FROM (
                        SELECT json_object('key', 'TIT2', 'value',
                                           json_object('type', 'text', 'text', 'Track ' || n.i)) AS x
                        UNION ALL
                        SELECT json_object('key', 'TPE1', 'value',
                                           json_object('type', 'text', 'text', 'Artist ' || (n.i % 90)))
                        WHERE n.i % 4 <> 0
                        UNION ALL
                        SELECT json_object('key', 'TCON', 'value',
                                           json_object('type', 'text', 'text', 'Tag ' || (n.i % 40)))
                        WHERE n.i % 3 <> 0 AND n.i % 5 <> 0
                        UNION ALL
                        SELECT json_object('key', 'APIC', 'value',
                                           json_object('type', 'binary', 'len', 50000))
                        WHERE n.i % 3 <> 0 AND n.i % 5 <> 0
                        UNION ALL
                        SELECT json_object('key', 'GEOB:Serato Markers2', 'value',
                                           json_object('type', 'binary', 'len', 600)))))
             FROM n;
             INSERT INTO recording_file (recording_id, file_id, role)
             SELECT id, id, 'best' FROM recording;
             INSERT INTO library_track (id, recording_id, linked_file_id, added_at)
             SELECT id, id, id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now') FROM recording;
             INSERT INTO crate_entry (crate_id, library_track_id)
             SELECT 1, id FROM library_track WHERE id % 5 < 2;
             INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, recording_id, relink_method)
             SELECT json_patch(json_object('TrackID', CAST(id AS TEXT),
                                'Location', 'file://localhost/C:/Music/' || id,
                                'Genre', 'RB ' || (id % 10), 'AverageBpm', '120.00',
                                'Name', 'Name ' || id),
                               CASE WHEN id % 7 <> 0
                                    THEN json_object('Artist', 'RB Artist ' || (id % 50))
                                    ELSE '{{}}' END),
                    'c:/music/' || id, '2026-09-30T10:00:00.000Z', id, id, 'path'
             FROM recording WHERE id % 3 = 0;
             INSERT INTO analysis (recording_id, source, energy)
             SELECT id, 'user', id % 10 + 1 FROM recording WHERE id % 4 = 0;
             INSERT INTO analysis (recording_id, source, energy)
             SELECT id, 'mik', id % 7 + 1 FROM recording WHERE id % 6 = 0;"
        ))?;
        tx.commit()?;
        crate::attach::attach(c).map(|_| ())
    });
}

fn check_a_library_of(count: i64) {
    let lib = Lib::new();
    build(&lib, count);

    let started = std::time::Instant::now();
    let o = lib.overview();
    println!("overview of {count} tracks: {:?}", started.elapsed());
    let timed = |what: &str, f: fn(&Connection) -> rusqlite::Result<()>| {
        let at = std::time::Instant::now();
        lib.run(move |c| f(c));
        println!("  {what}: {:?}", at.elapsed());
    };
    timed("library numbers", |c| {
        c.query_row(&library_sql(), ["2000"], |_| Ok(()))
    });
    timed("all music", |c| c.query_row(ALL_MUSIC, [], |_| Ok(())));
    timed("energy statement", |c| {
        c.prepare(ENERGY)?.query_map([], |_| Ok(()))?.for_each(drop);
        Ok(())
    });
    timed("genres statement", |c| {
        c.prepare(&genres_sql())?
            .query_map([], |_| Ok(()))?
            .for_each(drop);
        Ok(())
    });
    timed("genres", |c| genres(c, 0).map(|_| ()));
    timed("missing tags", |c| missing_tags(c, &NoVolumes).map(|_| ()));
    timed("missing art", |c| c.query_row(MISSING_ART, [], |_| Ok(())));
    timed("energy", |c| energy(c).map(|_| ()));

    let all = 1..=count;
    // The user's energy wins over Mixed In Key's; the rest have none.
    let shown_energy = |n: i64| match (n % 4 == 0, n % 6 == 0) {
        (true, _) => Some(n % 10 + 1),
        (false, true) => Some(n % 7 + 1),
        _ => None,
    };
    for level in 1..=10 {
        assert_eq!(
            i64::from(o.energy[level as usize - 1]),
            all.clone()
                .filter(|n| shown_energy(*n) == Some(level))
                .count() as i64,
            "tracks with energy {level}"
        );
    }
    assert_eq!(
        i64::from(o.no_energy),
        all.clone().filter(|n| shown_energy(*n).is_none()).count() as i64
    );
    // Every track is in the Library, so none is outside it.
    assert_eq!(o.outside_library, 0);
    assert_eq!(i64::from(o.tracks), count);
    assert_eq!(
        o.total_length_ms as i64,
        all.clone().map(|n| 1000 + (n % 7) * 1000).sum::<i64>()
    );
    assert_eq!(
        i64::from(o.in_crates),
        all.clone().filter(|n| n % 5 < 2).count() as i64
    );
    assert_eq!(
        i64::from(o.analyzed_by_rekordbox),
        all.clone().filter(|n| n % 3 == 0).count() as i64
    );
    assert_eq!(i64::from(o.all_music), count);
    if count <= 5_000 {
        // The short way of counting missing tags is the long way.
        let long = lib.run(|c| missing_tag_ids(c, &NoVolumes, true)).len();
        assert_eq!(o.missing_tags as usize, long);
    }
    // Known tracks (every third) take rekordbox's artist, absent for every
    // seventh; the rest take the tag's, absent for every fourth. The title is
    // always there (a tag or, at worst, the file's name).
    assert_eq!(
        i64::from(o.missing_tags),
        all.clone()
            .filter(|n| {
                let no_artist = if n % 3 == 0 { n % 7 == 0 } else { n % 4 == 0 };
                no_artist || made_up_genre(*n).is_none()
            })
            .count() as i64
    );
    assert_eq!(
        i64::from(o.missing_art),
        all.clone().filter(|n| !(n % 3 != 0 && n % 5 != 0)).count() as i64
    );
    assert_eq!(i64::from(o.added_this_week), count);
    // Every file's verdict is missing, so none is counted; with no quality
    // job queued, nothing is counting.
    assert_eq!(o.low_quality, 0);
    assert!(!o.counting.quality);

    let mut expected: BTreeMap<String, u32> = BTreeMap::new();
    for n in all {
        if let Some(genre) = made_up_genre(n) {
            *expected.entry(genre).or_default() += 1;
        }
    }
    let mut expected: Vec<(String, u32)> = expected.into_iter().collect();
    expected.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let got: Vec<(String, u32)> = o
        .genres
        .iter()
        .map(|g| (g.name.clone(), g.tracks))
        .collect();
    assert_eq!(got, expected);
    assert_eq!(
        i64::from(o.no_genre),
        count - expected.iter().map(|g| i64::from(g.1)).sum::<i64>()
    );

    // Grouping happens in SQL: the genres statement returns a row per
    // distinct source, not a row per track.
    let rows = lib.run(|c| Ok(c.prepare(&genres_sql())?.query_map([], |_| Ok(()))?.count()));
    assert!(rows < 200, "{rows} rows for {count} tracks");
}

#[test]
fn a_library_of_5000_tracks_gives_the_right_numbers() {
    check_a_library_of(5_000);
}

#[test]
#[ignore = "builds 100,000 tracks; run with --release -- --ignored --nocapture"]
fn a_library_of_100000_tracks_gives_the_right_numbers() {
    check_a_library_of(100_000);
}

#[test]
fn the_frontend_gets_every_number_over_ipc() {
    use crate::ipc::testing::{app, invoke};
    use tauri::Manager;

    let (_data, app) = app();
    app.state::<Writer>()
        .call(|c| {
            c.execute_batch(
                "INSERT INTO volume (identity, kind, last_mount_path)
                 VALUES ('serial=1A2B3C4D', 'internal', 'C:\');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                 VALUES (1, 'Music', 'Music');
                 INSERT INTO file (music_folder_id, rel_path, rel_path_key, duration_ms)
                 VALUES (1, 'a.mp3', 'a.mp3', 90000);
                 INSERT INTO recording DEFAULT VALUES;
                 INSERT INTO recording_file (recording_id, file_id, role) VALUES (1, 1, 'best');
                 INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, 1);",
            )
        })
        .unwrap();

    let o = invoke(&app, "overview", json!({})).unwrap();
    assert_eq!(o["tracks"], 1);
    assert_eq!(o["totalLengthMs"], 90_000);
    assert_eq!(o["allMusic"], 1);
    assert_eq!(o["addedThisWeek"], 1);
    assert_eq!(o["lastSentMs"], Value::Null);
    assert_eq!(o["rekordboxRead"], false);
    assert_eq!(o["genres"], json!([]));
    assert_eq!(o["noGenre"], 1);
    assert_eq!(o["energy"], json!([0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    assert_eq!(o["noEnergy"], 1);
    assert_eq!(o["outsideLibrary"], 0);
    assert!(o["counting"]["files"].is_boolean());
}
