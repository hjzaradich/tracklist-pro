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
    let best = lib.file(track, "best.flac", "best", id3(&[("TIT2", "Best title")]));
    let linked = lib.file(
        track,
        "linked.mp3",
        "undecided",
        id3(&[("TIT2", "Linked title")]),
    );
    let library = lib.library(track, Some(linked));
    let _ = best;

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
    let _extra = lib.file(
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
    let later = lib.file(
        track,
        "later.mp3",
        "undecided",
        id3(&[("TCON", "Genre from later")]),
    );
    let _ = later;
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
fn a_missing_file_that_rekordbox_knows_still_cannot_be_sent() {
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
    lib.rekordbox(file, &[("Name", "N")], false);
    let library = lib.library(track, Some(file));
    assert_eq!(
        lib.one(library).outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: file })
    );
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
fn an_empty_batch_gives_nothing() {
    let lib = Lib::new();
    assert!(lib.values(&[]).is_empty());
}
