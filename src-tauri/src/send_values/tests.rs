//! Per-track send values (1aD-8). Everything is synthetic: a made-up
//! volume, folders, files, tags and rekordbox rows in a migrated database
//! in a temp dir.

use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};

use super::*;
use crate::db::Writer;
use crate::paths::Volumes;
use crate::volume::{identity, IdentitySignals, Volume, VolumeId, VolumeKind};

fn volume_id() -> VolumeId {
    identity(IdentitySignals {
        kind: VolumeKind::External,
        unc_share: None,
        serial: Some(0x1A2B_3C4D),
        filesystem: "NTFS",
        guid: None,
    })
    .unwrap()
}

/// The test volume, mounted at `0` or (if `None`) unplugged.
struct Mount(Option<PathBuf>);

impl Volumes for Mount {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up in these tests",
            path.display()
        )))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        self.0.clone().filter(|_| *id == volume_id())
    }
}

fn plugged() -> Mount {
    Mount(Some(PathBuf::from(r"E:\")))
}

struct Lib {
    _dir: tempfile::TempDir,
    writer: Writer,
    next_track_id: std::cell::Cell<i64>,
}

/// What a test file is like. Facts and tags are only set when a test cares.
#[derive(Default, Clone)]
struct Spec {
    present: Option<bool>,
    size: Option<i64>,
    format: Option<&'static str>,
    bitrate: Option<i64>,
    sample_rate: Option<i64>,
    duration_ms: Option<i64>,
    id3v2: Vec<(&'static str, &'static str)>,
}

fn id3(frames: &[(&'static str, &'static str)]) -> Spec {
    Spec {
        id3v2: frames.to_vec(),
        ..Spec::default()
    }
}

impl Lib {
    fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        let identity = volume_id().as_str().to_owned();
        writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES (?1, 'external', 'E:\\')",
                    [identity],
                )?;
                c.execute_batch(
                    "INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (1, 'Music', 'Music');",
                )
            })
            .unwrap();
        Lib {
            _dir: dir,
            writer,
            next_track_id: std::cell::Cell::new(1),
        }
    }

    fn insert(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, params)?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    fn track(&self) -> i64 {
        self.insert("INSERT INTO recording DEFAULT VALUES", ())
    }

    /// A file of `track`.
    fn file(&self, track: i64, name: &str, role: &str, spec: Spec) -> i64 {
        let raw_tags = (!spec.id3v2.is_empty()).then(|| {
            let items: Vec<Json> = spec
                .id3v2
                .iter()
                .map(|(key, text)| json!({"key": key, "value": {"type": "text", "text": text}}))
                .collect();
            json!({ "id3v2": items }).to_string()
        });
        let file = self.insert(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, size,
                               sniffed_format, bitrate, sample_rate, duration_ms, raw_tags)
             VALUES (1, ?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                name.to_owned(),
                spec.present.unwrap_or(true),
                spec.size,
                spec.format,
                spec.bitrate,
                spec.sample_rate,
                spec.duration_ms,
                raw_tags,
            ),
        );
        self.insert(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, ?3)",
            (track, file, role.to_owned()),
        );
        file
    }

    /// A rekordbox entry matched to `file` with these attributes, in order.
    fn rekordbox(&self, file: i64, attributes: &[(&str, &str)], probable: bool) -> i64 {
        let track_id = self.next_track_id.get();
        self.next_track_id.set(track_id + 1);
        // Built by hand, in this order: a JSON map would sort the names.
        let mut parts = vec![format!("\"TrackID\":\"{track_id}\"")];
        let mut all: Vec<(String, String)> = attributes
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        if !all.iter().any(|(k, _)| k == "Location") {
            all.push((
                "Location".into(),
                format!("file://localhost/E:/Music/{track_id}.mp3"),
            ));
        }
        for (k, v) in &all {
            parts.push(format!(
                "{}:{}",
                serde_json::to_string(k).unwrap(),
                serde_json::to_string(v).unwrap()
            ));
        }
        let attributes = format!("{{{}}}", parts.join(","));
        self.insert(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, relink_method, relink_probable)
             VALUES (?1, ?2, '2026-09-30T10:00:00.000Z', ?3, ?4, ?5)",
            (
                attributes,
                format!("E:/Music/{track_id}.mp3"),
                file,
                if probable { "filename_only" } else { "path" },
                probable,
            ),
        );
        track_id
    }

    /// A rekordbox entry at `E:\Music\<name>` (the Location a Library track
    /// linked to that file would have), keyed by its Location the way a read
    /// keys it, not matched to any file unless `file` says so. `read_at` is
    /// the read it belongs to.
    fn rekordbox_at(
        &self,
        name: &str,
        file: Option<i64>,
        probable: bool,
        read_at: &str,
        attributes: &[(&str, &str)],
    ) -> i64 {
        let track_id = self.next_track_id.get();
        self.next_track_id.set(track_id + 1);
        let location = format!("file://localhost/E:/Music/{name}");
        let mut parts = vec![format!("\"TrackID\":\"{track_id}\"")];
        for (k, v) in attributes.iter().chain(&[("Location", location.as_str())]) {
            parts.push(format!(
                "{}:{}",
                serde_json::to_string(k).unwrap(),
                serde_json::to_string(v).unwrap()
            ));
        }
        self.insert(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, relink_method, relink_probable)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                format!("{{{}}}", parts.join(",")),
                crate::rekordbox::location::decode(&location)
                    .unwrap()
                    .match_key(),
                read_at.to_owned(),
                file,
                file.map(|_| if probable { "filename_only" } else { "path" }),
                probable,
            ),
        )
    }

    /// A Library track for `track`, linked to `file` (or to nothing).
    fn library(&self, track: i64, file: Option<i64>) -> LibraryTrackId {
        LibraryTrackId(self.insert(
            "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
             VALUES (?1, 'linked', ?2, ?3)",
            (track, file, if file.is_some() { "ok" } else { "missing" }),
        ))
    }

    /// A Library track linked to a new file of a new track.
    fn linked(&self, name: &str, spec: Spec) -> (LibraryTrackId, i64) {
        let track = self.track();
        let file = self.file(track, name, "best", spec);
        (self.library(track, Some(file)), file)
    }

    fn values(&self, ids: &[LibraryTrackId]) -> Vec<SendValues> {
        let ids = ids.to_vec();
        self.writer
            .call(move |c| send_values(c, &plugged(), &ids))
            .unwrap()
    }

    fn one(&self, id: LibraryTrackId) -> SendValues {
        self.writer
            .call(move |c| send_values_one(c, &plugged(), id))
            .unwrap()
    }

    fn ready(&self, id: LibraryTrackId) -> TrackValues {
        match self.one(id).outcome {
            Outcome::Ready(values) => values,
            other => panic!("expected values, got {other:?}"),
        }
    }
}

fn pairs(values: &TrackValues) -> Vec<(&str, &str)> {
    values
        .values
        .iter()
        .map(|v| (v.attribute.as_str(), v.value.as_str()))
        .collect()
}

fn value<'a>(values: &'a TrackValues, attribute: &str) -> Option<&'a str> {
    values.get(attribute).map(|v| v.value.as_str())
}

// ---- a track rekordbox knows ---------------------------------------------------

#[test]
fn a_track_rekordbox_knows_returns_its_values_untouched_and_says_it_is_already_there() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(
        track,
        "a.mp3",
        "best",
        id3(&[("TIT2", "A different title")]),
    );
    // Strings as read: an empty one, a zero, spaces, an entity's result,
    // and the analysis fields.
    let attributes = [
        ("Name", "Kit & Kin's \"dub\""),
        ("Artist", ""),
        ("Size", "0"),
        ("AverageBpm", "128.00"),
        ("Tonality", "F#m"),
        ("PlayCount", "0"),
        ("Comments", "  spaced \t tab /* TLP */"),
        ("Location", "file://localhost/E:/Music/a%20b.mp3"),
    ];
    let known = lib.rekordbox(file, &attributes, false);
    let library = lib.library(track, Some(file));

    let values = lib.ready(library);
    assert!(values.in_rekordbox);
    let mut expected = vec![("TrackID", known.to_string())];
    expected.extend(attributes.iter().map(|(k, v)| (*k, (*v).to_string())));
    let got: Vec<(&str, String)> = pairs(&values)
        .into_iter()
        .map(|(k, v)| (k, v.to_string()))
        .collect();
    assert_eq!(got, expected);
    assert!(values
        .values
        .iter()
        .all(|v| v.provenance == Provenance::Rekordbox { track_id: known }));
    assert!(values.disagreements.is_empty());
    // The analysis fields are in the values, for the writer to leave out.
    for name in ANALYSIS_ATTRIBUTES {
        assert!(values.get(name).is_some(), "{name}");
    }
}

#[test]
fn a_probable_match_does_not_make_a_track_known() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(track, "a.mp3", "best", id3(&[("TIT2", "From the tag")]));
    lib.rekordbox(
        file,
        &[("Name", "From rekordbox"), ("PlayCount", "9")],
        true,
    );
    let library = lib.library(track, Some(file));

    let values = lib.ready(library);
    assert!(!values.in_rekordbox);
    assert_eq!(value(&values, "Name"), Some("From the tag"));
    assert!(values.get("PlayCount").is_none());
    assert!(values.get("TrackID").is_none());
}

#[test]
fn the_most_played_entry_is_the_one_known_when_rekordbox_holds_the_file_twice() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(track, "a.mp3", "best", Spec::default());
    lib.rekordbox(file, &[("Name", "Less played"), ("PlayCount", "1")], false);
    let more = lib.rekordbox(file, &[("Name", "More played"), ("PlayCount", "5")], false);
    let library = lib.library(track, Some(file));

    let values = lib.ready(library);
    assert_eq!(value(&values, "Name"), Some("More played"));
    assert_eq!(value(&values, "TrackID"), Some(more.to_string().as_str()));
}

// ---- a track rekordbox doesn't know -----------------------------------------------

#[test]
fn a_tag_value_comes_from_the_linked_file_first() {
    let lib = Lib::new();
    let track = lib.track();
    lib.file(track, "best.flac", "best", id3(&[("TIT2", "Best title")]));
    let linked = lib.file(
        track,
        "linked.mp3",
        "undecided",
        id3(&[("TIT2", "Linked title")]),
    );
    let library = lib.library(track, Some(linked));

    let values = lib.ready(library);
    assert_eq!(value(&values, "Name"), Some("Linked title"));
    assert_eq!(
        values.get("Name").unwrap().provenance,
        Provenance::FileTag {
            file_id: linked,
            tag: "id3v2:TIT2".into()
        }
    );
}

#[test]
fn a_field_the_linked_file_lacks_comes_from_the_other_files_in_best_file_order() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(track, "linked.mp3", "undecided", id3(&[("TIT2", "T")]));
    // Lower id, but not the best: it comes after the best file.
    lib.file(
        track,
        "extra.mp3",
        "extra",
        id3(&[("TALB", "Album from extra")]),
    );
    let best = lib.file(
        track,
        "best.flac",
        "best",
        id3(&[("TALB", "Album from best"), ("TCON", "Genre from best")]),
    );
    lib.file(
        track,
        "later.mp3",
        "undecided",
        id3(&[("TCON", "Genre from later")]),
    );
    let library = lib.library(track, Some(linked));

    let values = lib.ready(library);
    assert_eq!(value(&values, "Album"), Some("Album from best"));
    assert_eq!(value(&values, "Genre"), Some("Genre from best"));
    assert_eq!(
        values.get("Album").unwrap().provenance,
        Provenance::FileTag {
            file_id: best,
            tag: "id3v2:TALB".into()
        }
    );
}

#[test]
fn with_no_best_among_the_others_the_lowest_file_id_comes_first() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(track, "linked.mp3", "undecided", Spec::default());
    lib.file(track, "a.mp3", "undecided", id3(&[("TALB", "Lower id")]));
    lib.file(track, "b.mp3", "extra", id3(&[("TALB", "Higher id")]));
    let library = lib.library(track, Some(linked));
    assert_eq!(value(&lib.ready(library), "Album"), Some("Lower id"));
}

#[test]
fn a_blank_tag_in_the_linked_file_falls_back_to_the_others() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(track, "linked.mp3", "best", id3(&[("TALB", "   ")]));
    lib.file(track, "other.mp3", "extra", id3(&[("TALB", "Real album")]));
    let library = lib.library(track, Some(linked));
    assert_eq!(value(&lib.ready(library), "Album"), Some("Real album"));
}

#[test]
fn files_that_are_not_on_disk_are_not_asked_for_tags() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(track, "linked.mp3", "undecided", Spec::default());
    lib.file(
        track,
        "gone.mp3",
        "best",
        Spec {
            present: Some(false),
            ..id3(&[("TALB", "From a missing file")])
        },
    );
    let library = lib.library(track, Some(linked));
    assert_eq!(value(&lib.ready(library), "Album"), None);
}

#[test]
fn file_facts_come_from_the_linked_file_alone_and_the_location_is_its_plain_path() {
    let lib = Lib::new();
    let track = lib.track();
    let facts = |size, ms, bitrate, rate| Spec {
        size: Some(size),
        format: Some("mp3"),
        bitrate: Some(bitrate),
        sample_rate: Some(rate),
        duration_ms: Some(ms),
        ..Spec::default()
    };
    let linked = lib.file(
        track,
        "My Track #1.mp3",
        "undecided",
        facts(1000, 255_634, 320, 44_100),
    );
    lib.file(track, "best.flac", "best", {
        let mut s = facts(9, 9_000, 1411, 96_000);
        s.format = Some("flac");
        s
    });
    let library = lib.library(track, Some(linked));

    let values = lib.ready(library);
    for (attribute, expected) in [
        ("Kind", "MP3 File"),
        ("Size", "1000"),
        // Whole seconds, truncated, not rounded.
        ("TotalTime", "255"),
        ("BitRate", "320"),
        ("SampleRate", "44100"),
        ("Location", r"E:\Music\My Track #1.mp3"),
    ] {
        assert_eq!(value(&values, attribute), Some(expected), "{attribute}");
        assert_eq!(
            values.get(attribute).unwrap().provenance,
            Provenance::FileFact { file_id: linked }
        );
    }
}

#[test]
fn a_file_on_an_unplugged_drive_still_gets_a_location_from_where_it_was_last_seen() {
    let lib = Lib::new();
    let (library, _) = lib.linked("a.mp3", Spec::default());
    let unplugged = lib
        .writer
        .call(move |c| send_values_one(c, &Mount(None), library))
        .unwrap();
    let Outcome::Ready(values) = unplugged.outcome else {
        panic!("{unplugged:?}");
    };
    assert_eq!(value(&values, "Location"), Some(r"E:\Music\a.mp3"));
}

#[test]
fn a_format_rekordbox_does_not_play_gets_no_kind() {
    let lib = Lib::new();
    let (library, _) = lib.linked(
        "a.opus",
        Spec {
            format: Some("opus"),
            ..Spec::default()
        },
    );
    assert_eq!(value(&lib.ready(library), "Kind"), None);
}

#[test]
fn a_track_rekordbox_does_not_know_never_gets_analysis_or_rekordbox_owned_fields() {
    let lib = Lib::new();
    let track = lib.track();
    // Tags that carry a BPM and a key, and a tag-sourced analysis row.
    let file = lib.file(
        track,
        "a.mp3",
        "best",
        id3(&[
            ("TIT2", "T"),
            ("TBPM", "128"),
            ("TKEY", "Am"),
            ("TXXX:ENERGY", "7"),
        ]),
    );
    lib.insert(
        "INSERT INTO analysis (recording_id, source, bpm, key) VALUES (?1, 'tag', 128, '8A')",
        (track,),
    );
    let library = lib.library(track, Some(file));

    let values = lib.ready(library);
    for name in [
        "TrackID",
        "AverageBpm",
        "Tonality",
        "Bpm",
        "PlayCount",
        "LastPlayed",
        "Rating",
        "Colour",
        "Grouping",
        "Mix",
        "DateAdded",
        "DateModified",
    ] {
        assert!(values.get(name).is_none(), "{name}");
    }
    assert_eq!(value(&values, "Name"), Some("T"));
}

#[test]
fn the_values_come_in_the_order_rekordbox_writes_them() {
    let lib = Lib::new();
    let (library, _) = lib.linked(
        "a.mp3",
        Spec {
            size: Some(1),
            format: Some("mp3"),
            duration_ms: Some(2000),
            id3v2: vec![("TPUB", "L"), ("TIT2", "T"), ("TALB", "A")],
            ..Spec::default()
        },
    );
    let values = lib.ready(library);
    let names: Vec<&str> = values.values.iter().map(|v| v.attribute.as_str()).collect();
    assert_eq!(
        names,
        [
            "Name",
            "Album",
            "Kind",
            "Size",
            "TotalTime",
            "Location",
            "Label"
        ]
    );
}

// ---- files that disagree ------------------------------------------------------------

#[test]
fn files_that_disagree_still_give_one_value_by_the_rule_and_the_disagreement_is_listed() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(
        track,
        "linked.mp3",
        "undecided",
        id3(&[("TPE1", "Artist One"), ("TALB", "Same Album")]),
    );
    let best = lib.file(
        track,
        "best.flac",
        "best",
        id3(&[("TPE1", "Artist Two"), ("TALB", "Same Album")]),
    );
    let library = lib.library(track, Some(linked));

    let values = lib.ready(library);
    assert_eq!(value(&values, "Artist"), Some("Artist One"));
    assert_eq!(
        values.disagreements,
        vec![Disagreement {
            attribute: "Artist".into(),
            values: vec![
                FileValue {
                    file_id: linked,
                    value: "Artist One".into()
                },
                FileValue {
                    file_id: best,
                    value: "Artist Two".into()
                },
            ],
        }]
    );
}

#[test]
fn a_fallback_value_that_no_other_file_contradicts_is_not_a_disagreement() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(track, "linked.mp3", "undecided", Spec::default());
    lib.file(track, "a.mp3", "best", id3(&[("TALB", "Only one says")]));
    let library = lib.library(track, Some(linked));
    assert!(lib.ready(library).disagreements.is_empty());
}

// ---- a file that is missing --------------------------------------------------------------

#[test]
fn a_track_whose_linked_file_is_missing_cannot_be_sent_and_the_others_still_are() {
    let lib = Lib::new();
    let (good, _) = lib.linked("good.mp3", id3(&[("TIT2", "Good")]));
    let (gone, gone_file) = lib.linked(
        "gone.mp3",
        Spec {
            present: Some(false),
            ..Spec::default()
        },
    );
    let none = lib.library(lib.track(), None);

    let answers = lib.values(&[gone, good, none, LibraryTrackId(9999)]);
    assert_eq!(
        answers[0].outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: gone_file })
    );
    assert!(matches!(&answers[1].outcome, Outcome::Ready(v) if value(v, "Name") == Some("Good")));
    assert_eq!(
        answers[2].outcome,
        Outcome::CannotSend(CannotSend::NoLinkedFile)
    );
    assert_eq!(
        answers[3].outcome,
        Outcome::CannotSend(CannotSend::NotInLibrary)
    );
}

#[test]
fn a_missing_file_that_rekordbox_knows_is_sent_as_rekordboxs_own_entry() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(
        track,
        "a.mp3",
        "best",
        Spec {
            present: Some(false),
            ..Spec::default()
        },
    );
    lib.rekordbox(file, &[("Name", "N"), ("Rating", "51")], false);
    let library = lib.library(track, Some(file));
    let values = lib.ready(library);
    assert!(values.in_rekordbox && values.file_missing);
    assert_eq!(value(&values, "Name"), Some("N"));
    assert_eq!(value(&values, "Rating"), Some("51"));
    assert!(values.disagreements.is_empty());
}

#[test]
fn a_missing_file_whose_rekordbox_match_is_only_probable_still_cannot_be_sent() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(
        track,
        "a.mp3",
        "best",
        Spec {
            present: Some(false),
            ..Spec::default()
        },
    );
    lib.rekordbox(file, &[("Name", "N")], true);
    let library = lib.library(track, Some(file));
    assert_eq!(
        lib.one(library).outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: file })
    );
}

#[test]
fn a_present_file_is_never_marked_missing() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(track, "a.mp3", "best", Spec::default());
    lib.rekordbox(file, &[("Name", "N")], false);
    let library = lib.library(track, Some(file));
    assert!(!lib.ready(library).file_missing);
    let (plain, _) = lib.linked("b.mp3", Spec::default());
    assert!(!lib.ready(plain).file_missing);
}

// ---- a missing file, found through its own Location --------------------------------------
//
// Relink never matches a rekordbox row to a missing file, so the row has no
// file; it's found by the file's own path.

const READ: &str = "2026-09-30T10:00:00.000Z";

/// A Library track linked to a missing `E:\Music\gone.mp3`.
fn gone(lib: &Lib) -> (LibraryTrackId, i64) {
    lib.linked(
        "gone.mp3",
        Spec {
            present: Some(false),
            ..Spec::default()
        },
    )
}

#[test]
fn a_missing_file_is_sent_as_the_rekordbox_entry_at_its_own_location_though_no_file_is_matched() {
    let lib = Lib::new();
    let (library, _) = gone(&lib);
    lib.rekordbox_at(
        "gone.mp3",
        None,
        false,
        READ,
        &[("Name", "Theirs"), ("Rating", "51")],
    );
    let values = lib.ready(library);
    assert!(values.in_rekordbox && values.file_missing);
    assert_eq!(value(&values, "Name"), Some("Theirs"));
    assert_eq!(value(&values, "Rating"), Some("51"));
    assert_eq!(
        value(&values, "Location"),
        Some("file://localhost/E:/Music/gone.mp3")
    );
}

#[test]
fn the_location_is_matched_the_way_relink_matches_a_path_so_letter_case_does_not_matter() {
    let lib = Lib::new();
    let (library, _) = lib.linked(
        "Gone Song.mp3",
        Spec {
            present: Some(false),
            ..Spec::default()
        },
    );
    lib.rekordbox_at("gone%20song.MP3", None, false, READ, &[("Name", "Theirs")]);
    assert!(lib.ready(library).file_missing);
}

#[test]
fn two_rekordbox_entries_at_the_location_are_ambiguous_so_the_track_is_left_out() {
    let lib = Lib::new();
    let (library, file) = gone(&lib);
    lib.rekordbox_at("gone.mp3", None, false, READ, &[("Name", "One")]);
    lib.rekordbox_at("gone.mp3", None, false, READ, &[("Name", "Two")]);
    assert_eq!(
        lib.one(library).outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: file })
    );
}

#[test]
fn an_entry_matched_to_a_different_file_is_not_used() {
    let lib = Lib::new();
    let (library, file) = gone(&lib);
    // rekordbox's row at that Location is already the entry of another
    // Library file (say a copy relink found by name and duration).
    let (_, other) = lib.linked("other.mp3", Spec::default());
    lib.rekordbox_at("gone.mp3", Some(other), false, READ, &[("Name", "Theirs")]);
    assert_eq!(
        lib.one(library).outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: file })
    );
}

#[test]
fn a_probable_entry_matched_to_another_file_is_not_used() {
    let lib = Lib::new();
    let (library, file) = gone(&lib);
    let (_, other) = lib.linked("other.mp3", Spec::default());
    lib.rekordbox_at("gone.mp3", Some(other), true, READ, &[("Name", "Theirs")]);
    assert_eq!(
        lib.one(library).outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: file })
    );
}

#[test]
fn an_entry_kept_from_an_earlier_read_is_not_used() {
    let lib = Lib::new();
    let (library, file) = gone(&lib);
    // An incomplete read keeps older rows whose TrackIDs belong to the
    // earlier read; only the newest read's rows count.
    lib.rekordbox_at(
        "gone.mp3",
        None,
        false,
        "2026-09-29T10:00:00.000Z",
        &[("Name", "Old")],
    );
    lib.rekordbox_at(
        "elsewhere.mp3",
        None,
        false,
        READ,
        &[("Name", "Newer read")],
    );
    assert_eq!(
        lib.one(library).outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: file })
    );
}

#[test]
fn a_missing_file_rekordbox_has_no_entry_at_is_still_left_out() {
    let lib = Lib::new();
    let (library, file) = gone(&lib);
    lib.rekordbox_at("another.mp3", None, false, READ, &[("Name", "Theirs")]);
    assert_eq!(
        lib.one(library).outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: file })
    );
}

#[test]
fn a_present_file_never_looks_for_an_entry_at_its_location() {
    let lib = Lib::new();
    let (library, _) = lib.linked("here.mp3", Spec::default());
    // Not matched by relink (so unknown): it's a new track, not the entry.
    lib.rekordbox_at("here.mp3", None, false, READ, &[("Name", "Theirs")]);
    assert!(!lib.ready(library).in_rekordbox);
}

// ---- the batch --------------------------------------------------------------------------

#[test]
fn the_batch_gives_what_each_single_track_gives_in_the_order_asked() {
    let lib = Lib::new();
    // A known track, an unknown one with several files, a probable one, a
    // missing one, and many plain ones, so a mix-up between tracks would show.
    let known = {
        let track = lib.track();
        let file = lib.file(track, "known.mp3", "best", id3(&[("TIT2", "Tag title")]));
        lib.rekordbox(file, &[("Name", "rekordbox title")], false);
        lib.library(track, Some(file))
    };
    let several = {
        let track = lib.track();
        let linked = lib.file(track, "s1.mp3", "undecided", id3(&[("TIT2", "S1")]));
        lib.file(
            track,
            "s2.mp3",
            "best",
            id3(&[("TIT2", "S2"), ("TALB", "Album S2")]),
        );
        lib.library(track, Some(linked))
    };
    let probable = {
        let track = lib.track();
        let file = lib.file(track, "p.mp3", "best", id3(&[("TIT2", "P")]));
        lib.rekordbox(file, &[("Name", "never used")], true);
        lib.library(track, Some(file))
    };
    let missing = lib
        .linked(
            "m.mp3",
            Spec {
                present: Some(false),
                ..Spec::default()
            },
        )
        .0;
    let mut ids = vec![known, several, probable, missing];
    for (n, title) in ["Plain 0", "Plain 1", "Plain 2", "Plain 3", "Plain 4"]
        .into_iter()
        .enumerate()
    {
        ids.push(
            lib.linked(&format!("plain{n}.mp3"), id3(&[("TIT2", title)]))
                .0,
        );
    }
    ids.reverse();
    ids.push(known);

    let batch = lib.values(&ids);
    assert_eq!(batch.len(), ids.len());
    for (answer, id) in batch.iter().zip(&ids) {
        assert_eq!(answer.library_track, *id);
        assert_eq!(answer, &lib.one(*id), "{id:?}");
    }
}

#[test]
fn a_trusted_entry_for_another_file_of_the_track_does_not_make_the_linked_file_known() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(
        track,
        "linked.mp3",
        "undecided",
        id3(&[("TIT2", "From the tag")]),
    );
    let sibling = lib.file(track, "sibling.mp3", "best", Spec::default());
    let held = lib.rekordbox(
        sibling,
        &[("Name", "rekordbox's own"), ("PlayCount", "7")],
        false,
    );
    let library = lib.library(track, Some(linked));

    let values = lib.ready(library);
    // The linked file would be a second entry beside the sibling's.
    assert!(!values.in_rekordbox);
    assert_eq!(value(&values, "Name"), Some("From the tag"));
    assert!(values.get("PlayCount").is_none());
    assert_eq!(
        values.rekordbox_holds_other_file,
        Some(SiblingEntry {
            file_id: sibling,
            track_id: held
        })
    );
}

#[test]
fn a_sibling_that_is_only_a_probable_match_is_not_reported() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(track, "linked.mp3", "undecided", Spec::default());
    let sibling = lib.file(track, "sibling.mp3", "best", Spec::default());
    lib.rekordbox(sibling, &[("Name", "N")], true);
    let library = lib.library(track, Some(linked));
    assert_eq!(lib.ready(library).rekordbox_holds_other_file, None);
}

#[test]
fn a_track_rekordbox_knows_reports_no_sibling_even_when_it_holds_two_files() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(track, "linked.mp3", "best", Spec::default());
    let sibling = lib.file(track, "sibling.mp3", "undecided", Spec::default());
    lib.rekordbox(linked, &[("Name", "A")], false);
    lib.rekordbox(sibling, &[("Name", "B")], false);
    let library = lib.library(track, Some(linked));
    let values = lib.ready(library);
    assert!(values.in_rekordbox);
    assert_eq!(values.rekordbox_holds_other_file, None);
}

#[test]
fn a_big_batch_of_different_tracks_is_answered_in_one_go_each_with_its_own_values() {
    const N: usize = 5000;
    let lib = Lib::new();
    // N Library tracks, each with a file of its own, made in one transaction.
    let ids: Vec<LibraryTrackId> = lib
        .writer
        .call(|c| {
            let tx = c.transaction()?;
            let mut ids = Vec::with_capacity(N);
            for n in 0..N {
                tx.execute("INSERT INTO recording DEFAULT VALUES", [])?;
                let track = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key, raw_tags)
                     VALUES (1, ?1, ?1, ?2)",
                    (
                        format!("f{n}.mp3"),
                        json!({"id3v2": [
                            {"key": "TIT2", "value": {"type": "text", "text": format!("Title {n}")}}
                        ]})
                        .to_string(),
                    ),
                )?;
                let file = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'best')",
                    (track, file),
                )?;
                tx.execute(
                    "INSERT INTO library_track (recording_id, linked_file_id) VALUES (?1, ?2)",
                    (track, file),
                )?;
                ids.push(LibraryTrackId(tx.last_insert_rowid()));
            }
            tx.commit()?;
            Ok(ids)
        })
        .unwrap();
    let distinct: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(distinct.len(), N);

    let answers = lib.values(&ids);
    assert_eq!(answers.len(), N);
    for (n, (answer, id)) in answers.iter().zip(&ids).enumerate() {
        assert_eq!(answer.library_track, *id);
        let Outcome::Ready(values) = &answer.outcome else {
            panic!("{answer:?}");
        };
        assert_eq!(value(values, "Name"), Some(format!("Title {n}").as_str()));
        assert_eq!(
            value(values, "Location"),
            Some(format!(r"E:\Music\f{n}.mp3").as_str())
        );
    }
}

#[test]
fn a_track_rekordbox_knows_does_not_need_its_linked_files_path_to_read_back() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(track, "a.mp3", "best", Spec::default());
    lib.rekordbox(
        file,
        &[
            ("Name", "rekordbox's own"),
            ("Location", "file://localhost/E:/Music/a.mp3"),
        ],
        false,
    );
    let known = lib.library(track, Some(file));
    let (unknown, _) = lib.linked("b.mp3", Spec::default());
    // The volume's identity no longer reads back, so no file has a path.
    lib.insert("UPDATE volume SET identity = ?1", ("dev=not-hex",));

    // rekordbox's own Location is sent, so the known track is still ready;
    // the unknown one has no Location to send.
    let answers = lib.values(&[known, unknown]);
    let Outcome::Ready(values) = &answers[0].outcome else {
        panic!("{:?}", answers[0]);
    };
    assert!(values.in_rekordbox);
    assert_eq!(
        value(values, "Location"),
        Some("file://localhost/E:/Music/a.mp3")
    );
    assert!(matches!(
        answers[1].outcome,
        Outcome::CannotSend(CannotSend::NoLocation { .. })
    ));
}

#[test]
fn an_empty_batch_gives_nothing() {
    let lib = Lib::new();
    assert!(lib.values(&[]).is_empty());
}

#[test]
fn a_probable_entry_matched_to_this_same_missing_file_is_not_used_either() {
    let lib = Lib::new();
    let (library, file) = gone(&lib);
    lib.rekordbox_at("gone.mp3", Some(file), true, READ, &[("Name", "Theirs")]);
    assert_eq!(
        lib.one(library).outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: file })
    );
}

/// Two known volumes, both last mounted at `E:\`: the test volume (1) and
/// another (2, with its own `Music` folder). Each is mounted or not.
struct TwoVolumes {
    first: Option<PathBuf>,
    second: Option<PathBuf>,
}

fn second_volume_id() -> VolumeId {
    identity(IdentitySignals {
        kind: VolumeKind::External,
        unc_share: None,
        serial: Some(0x5566_7788),
        filesystem: "NTFS",
        guid: None,
    })
    .unwrap()
}

impl Volumes for TwoVolumes {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!("{}", path.display())))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        if *id == volume_id() {
            self.first.clone()
        } else if *id == second_volume_id() {
            self.second.clone()
        } else {
            None
        }
    }
}

impl Lib {
    /// A second volume, last mounted at `E:\` like the first, with a
    /// `Music` folder (music folder 2).
    fn second_volume(&self) {
        let identity = second_volume_id().as_str().to_owned();
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES (?1, 'external', 'E:\')",
                    [identity],
                )?;
                c.execute(
                    "INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (2, 'Music', 'Music')",
                    [],
                )
            })
            .unwrap();
    }

    /// A Library track linked to a missing `name` in `Music` of music folder
    /// `folder`.
    fn missing_in(&self, folder: i64, name: &'static str) -> (LibraryTrackId, i64) {
        let track = self.track();
        let file = self.insert(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, size)
             VALUES (?1, ?2, ?2, 0, 1)",
            (folder, name.to_owned()),
        );
        self.insert(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'best')",
            (track, file),
        );
        (self.library(track, Some(file)), file)
    }

    fn one_with(&self, volumes: TwoVolumes, id: LibraryTrackId) -> Outcome {
        self.writer
            .call(move |c| send_values_one(c, &volumes, id))
            .unwrap()
            .outcome
    }
}

#[test]
fn a_missing_file_on_an_unplugged_volume_never_gets_the_entry_of_another_volume_at_the_same_drive_letter(
) {
    let lib = Lib::new();
    lib.second_volume();
    // Volume 1 is unplugged and was last at `E:\`; volume 2 is plugged in
    // at `E:\` and rekordbox's row at `E:/Music/gone.mp3` is its file.
    let (library, file) = lib.missing_in(1, "gone.mp3");
    lib.rekordbox_at(
        "gone.mp3",
        None,
        false,
        READ,
        &[("Name", "Volume 2's song")],
    );
    let volumes = TwoVolumes {
        first: None,
        second: Some(PathBuf::from(r"E:\")),
    };
    assert_eq!(
        lib.one_with(volumes, library),
        Outcome::CannotSend(CannotSend::FileMissing { file_id: file })
    );
}

#[test]
fn of_two_missing_files_that_share_a_drive_letter_only_the_one_on_the_plugged_volume_gets_the_entry(
) {
    let lib = Lib::new();
    lib.second_volume();
    let (on_first, first_file) = lib.missing_in(1, "gone.mp3");
    let (on_second, _) = lib.missing_in(2, "gone.mp3");
    lib.rekordbox_at("gone.mp3", None, false, READ, &[("Name", "Theirs")]);
    let volumes = || TwoVolumes {
        first: None,
        second: Some(PathBuf::from(r"E:\")),
    };
    assert_eq!(
        lib.one_with(volumes(), on_first),
        Outcome::CannotSend(CannotSend::FileMissing {
            file_id: first_file
        })
    );
    assert!(matches!(
        lib.one_with(volumes(), on_second),
        Outcome::Ready(v) if v.file_missing
    ));
}

#[test]
fn a_volume_last_mounted_alone_at_its_letter_is_still_found_while_unplugged() {
    let lib = Lib::new();
    let (library, _) = gone(&lib);
    lib.rekordbox_at("gone.mp3", None, false, READ, &[("Name", "Theirs")]);
    let unplugged = Lib::values_with(&lib, Mount(None), library);
    assert!(matches!(unplugged, Outcome::Ready(v) if v.file_missing));
}

#[test]
fn one_row_wanted_by_two_files_goes_to_neither_whatever_the_order() {
    let lib = Lib::new();
    // Two files whose names are one name in two Unicode forms: one key.
    let (composed, composed_file) = lib.missing_in(1, "Caf\u{e9}.mp3");
    let (decomposed, decomposed_file) = lib.missing_in(1, "Cafe\u{301}.mp3");
    lib.rekordbox_at("Caf%C3%A9.mp3", None, false, READ, &[("Name", "Theirs")]);
    for ids in [[composed, decomposed], [decomposed, composed]] {
        let answers = lib.values(&ids);
        for (answer, id) in answers.iter().zip(ids) {
            let file_id = if id == composed {
                composed_file
            } else {
                decomposed_file
            };
            assert_eq!(
                answer.outcome,
                Outcome::CannotSend(CannotSend::FileMissing { file_id })
            );
        }
    }
}

impl Lib {
    fn values_with(&self, volumes: Mount, id: LibraryTrackId) -> Outcome {
        self.writer
            .call(move |c| send_values_one(c, &volumes, id))
            .unwrap()
            .outcome
    }
}

// ---- the title and artist the lists show (1aG-9) ----------------------------

fn names(title: &str, artist: &str) -> Shown {
    Shown {
        title: title.to_owned(),
        artist: artist.to_owned(),
    }
}

type Snapshot = (
    Vec<(String, i64)>,
    Vec<(i64, Option<String>, Option<String>)>,
);

impl Lib {
    fn shown(&self, ids: &[LibraryTrackId]) -> Vec<Shown> {
        let wanted = ids.to_vec();
        let mut all = self
            .writer
            .call(move |c| shown(c, &plugged(), &wanted))
            .unwrap();
        ids.iter()
            .map(|id| all.remove(id).expect("a Library track has names"))
            .collect()
    }

    fn shown_one(&self, id: LibraryTrackId) -> Shown {
        self.shown(&[id]).remove(0)
    }

    fn recording_of(&self, id: LibraryTrackId) -> i64 {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT recording_id FROM library_track WHERE id = ?1",
                    [id.0],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    fn set_names(&self, recording: i64, title: Option<&str>, artist: Option<&str>) {
        let (title, artist) = (title.map(str::to_owned), artist.map(str::to_owned));
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE recording SET title = ?1, artist = ?2 WHERE id = ?3",
                    (title, artist, recording),
                )
            })
            .unwrap();
    }

    /// Every table's row count, and the whole recording table.
    fn snapshot(&self) -> Snapshot {
        self.writer
            .call(|c| {
                let tables: Vec<String> = c
                    .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                let mut counts = Vec::new();
                for table in tables {
                    let n = c.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| {
                        r.get(0)
                    })?;
                    counts.push((table, n));
                }
                let recordings = c
                    .prepare("SELECT id, title, artist FROM recording ORDER BY id")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                Ok((counts, recordings))
            })
            .unwrap()
    }
}

#[test]
fn a_track_rekordbox_knows_shows_rekordbox_title_and_artist() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(
        track,
        "a.mp3",
        "best",
        id3(&[("TIT2", "Tag title"), ("TPE1", "Tag artist")]),
    );
    lib.rekordbox(
        file,
        &[("Name", "Their title"), ("Artist", "Their artist")],
        false,
    );
    let library = lib.library(track, Some(file));
    assert_eq!(lib.shown_one(library), names("Their title", "Their artist"));
}

#[test]
fn a_new_track_shows_its_files_tags() {
    let lib = Lib::new();
    let (library, _) = lib.linked(
        "a.mp3",
        id3(&[("TIT2", "Tag title"), ("TPE1", "Tag artist")]),
    );
    assert_eq!(lib.shown_one(library), names("Tag title", "Tag artist"));
}

#[test]
fn a_new_track_takes_a_missing_tag_from_its_other_file() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(
        track,
        "a.mp3",
        "best",
        id3(&[("TIT2", "From the linked file")]),
    );
    lib.file(
        track,
        "b.mp3",
        "extra",
        id3(&[("TIT2", "From the other"), ("TPE1", "Other artist")]),
    );
    let library = lib.library(track, Some(linked));
    assert_eq!(
        lib.shown_one(library),
        names("From the linked file", "Other artist")
    );
}

#[test]
fn a_track_with_neither_shows_its_file_name_and_a_blank_artist() {
    let lib = Lib::new();
    let (library, _) = lib.linked("Sub folder/some tune.mp3", Spec::default());
    assert_eq!(lib.shown_one(library), names("some tune.mp3", ""));
}

#[test]
fn blank_text_says_nothing_so_the_file_name_and_no_artist_show() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(track, "a.mp3", "best", id3(&[("TIT2", "x")]));
    lib.rekordbox(file, &[("Name", "  "), ("Artist", "")], false);
    let library = lib.library(track, Some(file));
    assert_eq!(lib.shown_one(library), names("a.mp3", ""));
}

#[test]
fn a_set_recording_title_and_artist_win_over_what_a_send_writes() {
    let lib = Lib::new();
    let (known, known_file) = lib.linked("k.mp3", id3(&[("TIT2", "Tag")]));
    lib.rekordbox(
        known_file,
        &[("Name", "Theirs"), ("Artist", "Their artist")],
        false,
    );
    let (new, _) = lib.linked("n.mp3", id3(&[("TIT2", "Tag"), ("TPE1", "Tag artist")]));
    lib.set_names(
        lib.recording_of(known),
        Some("Own title"),
        Some("Own artist"),
    );
    // Only the title is set here: the artist is still what a send writes.
    lib.set_names(lib.recording_of(new), Some("Own title"), None);
    assert_eq!(lib.shown_one(known), names("Own title", "Own artist"));
    assert_eq!(lib.shown_one(new), names("Own title", "Tag artist"));
}

#[test]
fn a_track_a_send_cannot_send_shows_its_stored_tags_else_its_file_name() {
    let lib = Lib::new();
    // The file is gone and rekordbox doesn't know it.
    let gone = |frames: &[(&'static str, &'static str)]| Spec {
        present: Some(false),
        ..id3(frames)
    };
    let (tagged, _) = lib.linked(
        "gone.mp3",
        gone(&[("TIT2", "Remembered title"), ("TPE1", "Remembered artist")]),
    );
    let (untagged, _) = lib.linked("gone too.mp3", gone(&[]));
    // No linked file at all, and no other file.
    let track = lib.track();
    let linkless = lib.library(track, None);
    assert!(matches!(
        lib.one(tagged).outcome,
        Outcome::CannotSend(CannotSend::FileMissing { .. })
    ));
    assert_eq!(
        lib.shown(&[tagged, untagged, linkless]),
        vec![
            names("Remembered title", "Remembered artist"),
            names("gone too.mp3", ""),
            names("", ""),
        ]
    );
}

#[test]
fn shown_is_what_a_send_writes_for_every_kind_of_track() {
    let lib = Lib::new();
    let mut ids = Vec::new();
    let mut files = Vec::new();

    // Known to rekordbox: with an artist, without one, without a title, with
    // blanks, and with the file gone (rekordbox's own entry goes back).
    let known = |file: &str, attributes: &[(&str, &str)], spec: Spec| {
        let track = lib.track();
        let f = lib.file(track, file, "best", spec);
        lib.rekordbox(f, attributes, false);
        lib.library(track, Some(f))
    };
    let gone = Spec {
        present: Some(false),
        ..Spec::default()
    };
    for (file, attributes, spec) in [
        (
            "k1.mp3",
            vec![("Name", "Their title"), ("Artist", "Their artist")],
            id3(&[("TIT2", "Tag"), ("TPE1", "Tag")]),
        ),
        ("k2.mp3", vec![("Name", "Only a title")], Spec::default()),
        ("k3.mp3", vec![("Artist", "No title")], Spec::default()),
        (
            "k4.mp3",
            vec![("Name", "Pad "), ("Artist", "")],
            Spec::default(),
        ),
        (
            "k5.mp3",
            vec![("Name", "Gone but known"), ("Artist", "Still theirs")],
            gone,
        ),
    ] {
        ids.push(known(file, &attributes, spec));
        files.push(file);
    }
    // New: tags, only an artist, several values in a field, none.
    for (file, spec) in [
        ("n1.mp3", id3(&[("TIT2", "T"), ("TPE1", "A")])),
        ("n2.mp3", id3(&[("TPE1", "Only an artist")])),
        ("n3.mp3", id3(&[("TIT2", "a\0b"), ("TPE1", "c\0d")])),
        ("deep/n4.mp3", Spec::default()),
    ] {
        ids.push(lib.linked(file, spec).0);
        files.push(file);
    }
    // New, and its other file has the tags.
    let track = lib.track();
    let linked = lib.file(track, "n5.mp3", "best", Spec::default());
    lib.file(
        track,
        "n5b.mp3",
        "extra",
        id3(&[("TIT2", "Elsewhere"), ("TPE1", "Else")]),
    );
    ids.push(lib.library(track, Some(linked)));
    files.push("n5.mp3");

    // Only a probable match: rekordbox's entry isn't trusted, so the tags
    // are what a send writes.
    let track = lib.track();
    let f = lib.file(
        track,
        "p1.mp3",
        "best",
        id3(&[("TIT2", "Tag of a probable match"), ("TPE1", "Tag artist")]),
    );
    lib.rekordbox(
        f,
        &[("Name", "Not trusted"), ("Artist", "Not trusted")],
        true,
    );
    ids.push(lib.library(track, Some(f)));
    files.push("p1.mp3");

    // A sibling known to rekordbox: the linked file isn't, so the track is
    // new and the linked file's tags are written, not the sibling's entry.
    let track = lib.track();
    let linked = lib.file(
        track,
        "s1.mp3",
        "best",
        id3(&[
            ("TIT2", "Tag of the linked file"),
            ("TPE1", "Linked artist"),
        ]),
    );
    let sibling = lib.file(
        track,
        "s2.mp3",
        "extra",
        id3(&[("TIT2", "Tag of the sibling")]),
    );
    lib.rekordbox(
        sibling,
        &[
            ("Name", "Rekordbox holds the sibling"),
            ("Artist", "Theirs"),
        ],
        false,
    );
    ids.push(lib.library(track, Some(linked)));
    files.push("s1.mp3");

    // Files whose tags disagree: the linked file's value is the one sent,
    // and a field only the other file has comes from the other file.
    let track = lib.track();
    let linked = lib.file(
        track,
        "d1.mp3",
        "best",
        id3(&[("TIT2", "First file title")]),
    );
    lib.file(
        track,
        "d2.mp3",
        "extra",
        id3(&[
            ("TIT2", "Second file title"),
            ("TPE1", "Second file artist"),
        ]),
    );
    ids.push(lib.library(track, Some(linked)));
    files.push("d1.mp3");

    // The same, with the linked file not the track's best file: the linked
    // file still comes first.
    let track = lib.track();
    lib.file(
        track,
        "d3.mp3",
        "best",
        id3(&[("TIT2", "Best file title"), ("TPE1", "Best file artist")]),
    );
    let linked = lib.file(
        track,
        "d4.mp3",
        "extra",
        id3(&[("TIT2", "Linked extra title")]),
    );
    ids.push(lib.library(track, Some(linked)));
    files.push("d4.mp3");

    // Three files, each with its own title: the first in order is sent.
    let track = lib.track();
    let linked = lib.file(track, "d5.mp3", "best", id3(&[("TIT2", "Title one")]));
    lib.file(track, "d6.mp3", "extra", id3(&[("TIT2", "Title two")]));
    lib.file(track, "d7.mp3", "extra", id3(&[("TIT2", "Title three")]));
    ids.push(lib.library(track, Some(linked)));
    files.push("d5.mp3");

    let shown = lib.shown(&ids);
    let sent = lib.values(&ids);
    assert_eq!(shown.len(), files.len());
    for ((shown, sent), file) in shown.iter().zip(&sent).zip(files) {
        let Outcome::Ready(values) = &sent.outcome else {
            panic!("{file}: a send has values for it");
        };
        let written = |attribute: &str| value(values, attribute).filter(|v| !v.trim().is_empty());
        assert_eq!(
            shown,
            &Shown {
                title: written("Name").map_or_else(|| file_name(file).to_owned(), str::to_owned),
                artist: written("Artist").unwrap_or_default().to_owned(),
            },
            "{file}"
        );
    }
}

#[test]
fn listing_names_leaves_the_database_unchanged() {
    let lib = Lib::new();
    let (known, known_file) = lib.linked("k.mp3", id3(&[("TIT2", "Tag")]));
    lib.rekordbox(known_file, &[("Name", "Theirs")], false);
    let (new, _) = lib.linked("n.mp3", id3(&[("TIT2", "T"), ("TPE1", "A")]));
    let (gone, _) = lib.linked(
        "g.mp3",
        Spec {
            present: Some(false),
            ..Spec::default()
        },
    );
    let before = lib.snapshot();
    let shown = lib.shown(&[known, new, gone]);
    let recordings = lib
        .writer
        .call(|c| shown_recordings(c, &[1, 2, 3]))
        .unwrap();
    let removed = lib
        .writer
        .call(|c| {
            shown_removed(
                c,
                &[Removed {
                    recording_id: 1,
                    location_key: None,
                    path_name: None,
                }],
            )
        })
        .unwrap();
    assert_eq!(shown.len(), 3);
    assert_eq!(recordings.len(), 3);
    assert_eq!(removed.len(), 1);
    assert_eq!(lib.snapshot(), before);
    // And no title was written: the recordings are still untitled.
    assert!(before
        .1
        .iter()
        .all(|(_, title, artist)| title.is_none() && artist.is_none()));
}

#[test]
fn many_tracks_are_named_in_one_batch_each_by_its_own_values() {
    let lib = Lib::new();
    let titles: Vec<&'static str> = (0..60)
        .map(|i| &*Box::leak(format!("Title {i}").into_boxed_str()))
        .collect();
    let ids: Vec<LibraryTrackId> = titles
        .iter()
        .enumerate()
        .map(|(i, title)| lib.linked(&format!("t{i}.mp3"), id3(&[("TIT2", *title)])).0)
        .collect();
    for (shown, title) in lib.shown(&ids).iter().zip(titles) {
        assert_eq!(shown, &names(title, ""));
    }
}

#[test]
fn a_recording_with_no_library_track_shows_its_own_tags_best_file_first() {
    let lib = Lib::new();
    let tagged = lib.track();
    lib.file(tagged, "z.mp3", "extra", id3(&[("TIT2", "From the other")]));
    lib.file(
        tagged,
        "a.mp3",
        "best",
        id3(&[("TIT2", "From the best"), ("TPE1", "Best artist")]),
    );
    let bare = lib.track();
    lib.file(bare, "dir/bare.mp3", "best", Spec::default());
    let set = lib.track();
    lib.file(set, "s.mp3", "best", id3(&[("TIT2", "Tag")]));
    lib.set_names(set, Some("Own"), None);
    let mut all = lib
        .writer
        .call(move |c| shown_recordings(c, &[tagged, bare, set]))
        .unwrap();
    assert_eq!(
        all.remove(&tagged),
        Some(names("From the best", "Best artist"))
    );
    assert_eq!(all.remove(&bare), Some(names("bare.mp3", "")));
    assert_eq!(all.remove(&set), Some(names("Own", "")));
}

#[test]
fn a_removed_track_shows_rekordboxs_row_at_its_sent_location_else_tags_else_the_file_name() {
    let lib = Lib::new();
    let with_row = lib.track();
    lib.file(
        with_row,
        "a.mp3",
        "best",
        id3(&[("TIT2", "Tag"), ("TPE1", "Tag artist")]),
    );
    lib.rekordbox_at(
        "sent.mp3",
        None,
        false,
        READ,
        &[("Name", "Theirs"), ("Artist", "")],
    );
    let with_tags = lib.track();
    lib.file(
        with_tags,
        "b.mp3",
        "best",
        id3(&[("TIT2", "Tag"), ("TPE1", "Tag artist")]),
    );
    let bare = lib.track();
    lib.file(bare, "c.mp3", "best", Spec::default());
    let key = |name: &str| {
        crate::rekordbox::location::decode(&format!("file://localhost/E:/Music/{name}"))
            .unwrap()
            .match_key()
    };
    let removed = |recording: i64, location: Option<String>, name: Option<&str>| Removed {
        recording_id: recording,
        location_key: location,
        path_name: name.map(str::to_owned),
    };
    let asked = vec![
        removed(with_row, Some(key("sent.mp3")), Some("sent.mp3")),
        removed(with_tags, Some(key("not-held.mp3")), Some("not-held.mp3")),
        removed(bare, Some(key("not-held.mp3")), Some("not-held.mp3")),
        removed(bare, None, None),
    ];
    let answers = lib.writer.call(move |c| shown_removed(c, &asked)).unwrap();
    assert_eq!(
        answers,
        vec![
            // rekordbox's row says no artist: that's what the user will see.
            names("Theirs", ""),
            names("Tag", "Tag artist"),
            names("not-held.mp3", ""),
            names("c.mp3", ""),
        ]
    );
}

// ---- which rekordbox row names a removed track -------------------------------

impl Lib {
    /// A rekordbox row with this `TrackID` at `E:\Music\<name>`, from the
    /// read made at `read_at`, named `title`.
    fn row_at(&self, track_id: i64, file_name: &str, read_at: &str, title: &str) {
        let location = format!("file://localhost/E:/Music/{file_name}");
        let attributes = json!({
            "TrackID": track_id.to_string(), "Name": title, "Artist": "",
            "Location": location,
        })
        .to_string();
        let key = crate::rekordbox::location::decode(&location)
            .unwrap()
            .match_key();
        self.insert(
            "INSERT INTO rekordbox_track (attributes, location_key, read_at)
             VALUES (?1, ?2, ?3)",
            (attributes, key, read_at.to_owned()),
        );
    }

    /// The names shown for a track removed after being sent to
    /// `E:\Music\<file_name>`, which has these tags.
    fn removed_names(&self, file_name: &'static str, tagged: &'static str) -> Shown {
        let track = self.track();
        self.file(track, "x.mp3", "best", id3(&[("TIT2", tagged)]));
        let key =
            crate::rekordbox::location::decode(&format!("file://localhost/E:/Music/{file_name}"))
                .unwrap()
                .match_key();
        let asked = vec![Removed {
            recording_id: track,
            location_key: Some(key),
            path_name: Some(file_name.to_owned()),
        }];
        self.writer
            .call(move |c| shown_removed(c, &asked))
            .unwrap()
            .remove(0)
    }
}

const OLD_READ: &str = "2026-09-01T10:00:00.000Z";
const NEW_READ: &str = "2026-10-01T10:00:00.000Z";

#[test]
fn when_two_rows_of_one_read_share_the_sent_location_the_lowest_track_id_names_the_track() {
    let lib = Lib::new();
    // The higher TrackID is stored first, so it isn't just the first row.
    lib.row_at(9, "sent.mp3", NEW_READ, "Name of TrackID nine");
    lib.row_at(3, "sent.mp3", NEW_READ, "Name of TrackID three");
    lib.row_at(5, "sent.mp3", NEW_READ, "Name of TrackID five");
    assert_eq!(
        lib.removed_names("sent.mp3", "Tag"),
        names("Name of TrackID three", "")
    );
}

#[test]
fn a_removed_track_listed_through_a_row_kept_from_an_older_read_is_named_by_that_row() {
    let lib = Lib::new();
    // An incomplete read kept this row from an earlier read: the manual
    // removals list counts it, so it names the track too.
    lib.row_at(5, "sent.mp3", OLD_READ, "Name from the kept row");
    lib.row_at(6, "another.mp3", NEW_READ, "Another track");
    assert_eq!(
        lib.removed_names("sent.mp3", "Tag"),
        names("Name from the kept row", "")
    );
}
