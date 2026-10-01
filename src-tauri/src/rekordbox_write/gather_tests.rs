//! From the database to the file: [`gather`] over a migrated database in
//! a temp dir, then [`build`]. Everything is synthetic: a made-up volume,
//! files, tags and rekordbox rows.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::*;
use crate::db::Writer;
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

/// The test volume, mounted at `E:\`.
struct Plugged;

impl Volumes for Plugged {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up in these tests",
            path.display()
        )))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (*id == volume_id()).then(|| PathBuf::from(r"E:\"))
    }
}

struct Lib {
    _dir: tempfile::TempDir,
    writer: Writer,
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
        Lib { _dir: dir, writer }
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

    /// A file of `track` at `E:\Music\<name>`, with this ID3 title.
    fn file(&self, track: i64, name: &str, present: bool, title: Option<&str>) -> i64 {
        let raw_tags = title.map(|title| {
            json!({ "id3v2": [{"key": "TIT2", "value": {"type": "text", "text": title}}] })
                .to_string()
        });
        let file = self.insert(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, size,
                               sniffed_format, raw_tags)
             VALUES (1, ?1, ?1, ?2, 1234, 'mp3', ?3)",
            (name.to_owned(), present, raw_tags),
        );
        self.insert(
            // The track's first file is its best one.
            "INSERT INTO recording_file (recording_id, file_id, role)
             VALUES (?1, ?2, CASE WHEN EXISTS (SELECT 1 FROM recording_file
                                               WHERE recording_id = ?1)
                             THEN 'extra' ELSE 'best' END)",
            (track, file),
        );
        file
    }

    /// A rekordbox entry with these attributes in order, matched to `file`
    /// (or to nothing).
    fn rekordbox(&self, track_id: i64, file: Option<i64>, attributes: &[(&str, &str)]) {
        // Built by hand, in this order: a JSON map would sort the names.
        let mut parts = vec![format!("\"TrackID\":\"{track_id}\"")];
        for (k, v) in attributes {
            parts.push(format!(
                "{}:{}",
                serde_json::to_string(k).unwrap(),
                serde_json::to_string(v).unwrap()
            ));
        }
        self.insert(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, relink_method)
             VALUES (?1, ?2, '2026-09-30T10:00:00.000Z', ?3, ?4)",
            (
                format!("{{{}}}", parts.join(",")),
                attributes
                    .iter()
                    .find(|(k, _)| *k == "Location")
                    .and_then(|(_, v)| crate::rekordbox::location::decode(v).ok())
                    .map_or_else(|| format!("key-{track_id}"), |l| l.match_key()),
                file,
                file.map(|_| "path"),
            ),
        );
    }

    fn library(&self, track: i64, file: i64) -> LibraryTrackId {
        LibraryTrackId(self.insert(
            "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
             VALUES (?1, 'linked', ?2, 'ok')",
            (track, file),
        ))
    }

    fn gather(&self, tracks: &[LibraryTrackId], crates: Vec<Node>) -> SendInput {
        let tracks = tracks.to_vec();
        self.writer
            .call(move |c| gather(c, &Plugged, &tracks, crates, Vec::new()))
            .unwrap()
    }
}

fn attributes(track: &SentTrack) -> Vec<(&str, &str)> {
    track
        .attributes
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect()
}

#[test]
fn a_send_gathered_from_the_database_follows_every_rule_end_to_end() {
    let lib = Lib::new();
    // A track rekordbox has, with its analysis and awkward values.
    let known_track = lib.track();
    let known_file = lib.file(known_track, "known.mp3", true, Some("The file's own title"));
    lib.rekordbox(
        40,
        Some(known_file),
        &[
            ("Name", "Kit & Kin's \"dub\""),
            ("Artist", ""),
            ("AverageBpm", "128.00"),
            ("Comments", "tab\there /* TLP */"),
            ("PlayCount", "0"),
            ("Location", "file://localhost/E:/Music/known.mp3"),
            ("Tonality", "F#m"),
            ("FutureField", "kept"),
        ],
    );
    // rekordbox's highest TrackID belongs to a track that isn't sent.
    lib.rekordbox(
        9000,
        None,
        &[("Location", "file://localhost/E:/Elsewhere/x.mp3")],
    );
    let known = lib.library(known_track, known_file);
    // A track rekordbox doesn't know.
    let new_track = lib.track();
    let new_file = lib.file(new_track, "New & shiny #1.mp3", true, Some("Fresh <cut>"));
    let new = lib.library(new_track, new_file);
    // A track whose file is gone.
    let gone_track = lib.track();
    let gone_file = lib.file(gone_track, "gone.mp3", false, None);
    let gone = lib.library(gone_track, gone_file);

    let crates = vec![Node::Playlist {
        name: "Warm up".into(),
        entries: vec![gone, new, known],
    }];
    let input = lib.gather(&[known, new, gone], crates);
    assert_eq!(input.highest_rekordbox_track_id, 9000);
    let out = build(&input).unwrap();

    assert_eq!(out.sent.len(), 2);
    // Known: rekordbox's values in rekordbox's order, minus its analysis.
    assert_eq!(out.sent[0].library_track, known);
    assert_eq!(
        attributes(&out.sent[0]),
        [
            ("TrackID", "40"),
            ("Name", "Kit & Kin's \"dub\""),
            ("Artist", ""),
            ("Comments", "tab\there /* TLP */"),
            ("PlayCount", "0"),
            ("Location", "file://localhost/E:/Music/known.mp3"),
            ("FutureField", "kept"),
        ]
    );
    // New: numbered above rekordbox's highest, values from the file, the
    // path encoded, and no analysis.
    assert_eq!(out.sent[1].library_track, new);
    assert_eq!(out.sent[1].track_id, 9001);
    assert_eq!(
        attributes(&out.sent[1]),
        [
            ("TrackID", "9001"),
            ("Name", "Fresh <cut>"),
            ("Kind", "MP3 File"),
            ("Size", "1234"),
            (
                "Location",
                "file://localhost/E:/Music/New%20%26%20shiny%20%231.mp3"
            ),
        ]
    );
    // Gone: left out with its entry; the others keep their order.
    assert_eq!(
        out.left_out,
        [LeftOut {
            library_track: gone,
            reason: Reason::NoValues(CannotSend::FileMissing { file_id: gone_file }),
            entries: vec![DroppedEntry {
                path: vec!["Crates".into(), "Warm up".into()],
                position: 0
            }],
        }]
    );
    let read = RekordboxXml::parse(out.xml()).unwrap();
    let keys: Vec<&str> = read.playlists.playlists()[0]
        .1
        .entries
        .iter()
        .map(|e| e.key.as_str())
        .collect();
    assert_eq!(keys, ["9001", "40"]);
    assert_eq!(read.entries_without_track, 0);
    let text = std::str::from_utf8(out.xml()).unwrap();
    for word in [
        "AverageBpm",
        "Tonality",
        "TEMPO",
        "POSITION_MARK",
        "128.00",
        "F#m",
    ] {
        assert!(!text.contains(word), "{word} is in the file");
    }
}

#[test]
fn a_tag_value_xml_cannot_carry_keeps_that_track_out_of_a_gathered_send() {
    let lib = Lib::new();
    let bad_track = lib.track();
    let bad_file = lib.file(bad_track, "bad.mp3", true, Some("bell\u{7}title"));
    let bad = lib.library(bad_track, bad_file);
    let good_track = lib.track();
    let good_file = lib.file(good_track, "good.mp3", true, Some("Fine"));
    let good = lib.library(good_track, good_file);

    let out = build(&lib.gather(&[bad, good], Vec::new())).unwrap();
    assert_eq!(
        out.sent.iter().map(|t| t.library_track).collect::<Vec<_>>(),
        [good]
    );
    assert_eq!(
        out.left_out,
        [LeftOut {
            library_track: bad,
            reason: Reason::UncarriableCharacter {
                attribute: "Name".into()
            },
            entries: vec![],
        }]
    );
}

#[test]
fn a_duplicate_rekordbox_holds_is_carried_through_for_the_send_flow_to_warn_about() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(track, "linked.mp3", true, Some("Linked"));
    let other = lib.file(track, "other.mp3", true, Some("Other"));
    lib.rekordbox(
        77,
        Some(other),
        &[("Location", "file://localhost/E:/Music/other.mp3")],
    );
    let library = lib.library(track, linked);

    let out = build(&lib.gather(&[library], Vec::new())).unwrap();
    assert_eq!(out.sent.len(), 1);
    assert!(!out.sent[0].in_rekordbox);
    assert_eq!(
        out.sent[0].rekordbox_holds_other_file,
        Some(SiblingEntry {
            file_id: other,
            track_id: 77
        })
    );
    assert_eq!(
        out.sent[0].location(),
        "file://localhost/E:/Music/linked.mp3"
    );
}

#[test]
fn with_no_rekordbox_read_new_tracks_are_numbered_from_one() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(track, "a.mp3", true, None);
    let library = lib.library(track, file);
    let input = lib.gather(&[library, LibraryTrackId(999)], Vec::new());
    assert_eq!(input.highest_rekordbox_track_id, 0);
    let out = build(&input).unwrap();
    assert_eq!(out.sent[0].track_id, 1);
    assert_eq!(
        out.left_out,
        [LeftOut {
            library_track: LibraryTrackId(999),
            reason: Reason::NoValues(CannotSend::NotInLibrary),
            entries: vec![],
        }]
    );
}

#[test]
fn a_known_track_with_a_missing_file_is_written_byte_exact_and_stays_in_its_crate() {
    let lib = Lib::new();
    let kept_track = lib.track();
    let kept_file = lib.file(kept_track, "kept.mp3", true, Some("Present"));
    lib.rekordbox(
        1,
        Some(kept_file),
        &[("Location", "file://localhost/E:/Music/kept.mp3")],
    );
    let gone_track = lib.track();
    let gone_file = lib.file(gone_track, "gone.mp3", false, Some("File tag"));
    // rekordbox's spelling: raw `#` and parentheses, which the writer
    // would escape for a track whose file is there.
    let theirs = "file://localhost/E:/Music/Gone%20#1%20(live).mp3";
    lib.rekordbox(
        2,
        Some(gone_file),
        &[
            ("Name", "rekordbox's title"),
            ("AverageBpm", "126.00"),
            ("Rating", "204"),
            ("PlayCount", "7"),
            ("Location", theirs),
            ("Tonality", "5A"),
            ("FutureField", "kept"),
        ],
    );
    // A missing file rekordbox doesn't know stays out, with its reason.
    let unknown_track = lib.track();
    let unknown_file = lib.file(unknown_track, "unknown.mp3", false, None);
    let kept = lib.library(kept_track, kept_file);
    let gone = lib.library(gone_track, gone_file);
    let unknown = lib.library(unknown_track, unknown_file);

    let crates = vec![Node::Playlist {
        name: "Warm up".into(),
        entries: vec![kept, gone, unknown],
    }];
    let input = lib.gather(&[kept, gone, unknown], crates);
    let out = build(&input).unwrap();

    let sent = out
        .sent
        .iter()
        .find(|t| t.library_track == gone)
        .expect("the missing-file track is sent");
    assert!(sent.in_rekordbox && sent.file_missing);
    assert_eq!(
        attributes(sent),
        [
            ("TrackID", "2"),
            ("Name", "rekordbox's title"),
            ("Rating", "204"),
            ("PlayCount", "7"),
            ("Location", theirs),
            ("FutureField", "kept"),
        ]
    );
    let text = std::str::from_utf8(out.xml()).unwrap();
    assert!(text.contains(&format!("Location=\"{theirs}\"")));
    // It's in the crate, in its place; only the unknown track is missing.
    let read = RekordboxXml::parse(out.xml()).unwrap();
    let keys: Vec<&str> = read.playlists.playlists()[0]
        .1
        .entries
        .iter()
        .map(|e| e.key.as_str())
        .collect();
    assert_eq!(keys, ["1", "2"]);
    assert_eq!(
        out.left_out,
        [LeftOut {
            library_track: unknown,
            reason: Reason::NoValues(CannotSend::FileMissing {
                file_id: unknown_file
            }),
            entries: vec![DroppedEntry {
                path: vec!["Crates".into(), "Warm up".into()],
                position: 2
            }],
        }]
    );
    // The same track with its file present gets the writer's spelling.
    lib.writer
        .call(move |c| c.execute("UPDATE file SET present = 1 WHERE id = ?1", [gone_file]))
        .unwrap();
    let out = build(&lib.gather(
        &[gone],
        vec![Node::Playlist {
            name: "Warm up".into(),
            entries: vec![gone],
        }],
    ))
    .unwrap();
    assert!(!out.sent[0].file_missing);
    assert_eq!(
        out.sent[0].location(),
        "file://localhost/E:/Music/Gone%20%231%20%28live%29.mp3"
    );
}

#[test]
fn a_missing_file_is_found_by_its_own_location_and_its_entry_is_written_byte_exact() {
    // The real flow: relink never matches a row to a missing file, so the
    // row has no file and is found at the file's own Location.
    let lib = Lib::new();
    let gone_track = lib.track();
    let gone_file = lib.file(gone_track, "Gone #1 (live).mp3", false, Some("File tag"));
    let theirs = "file://localhost/E:/Music/Gone%20#1%20(live).mp3";
    lib.rekordbox(
        2,
        None,
        &[
            ("Name", "rekordbox's title"),
            ("AverageBpm", "126.00"),
            ("Rating", "204"),
            ("Location", theirs),
            ("Tonality", "5A"),
            ("FutureField", "kept"),
        ],
    );
    let gone = lib.library(gone_track, gone_file);
    let out = build(&lib.gather(
        &[gone],
        vec![Node::Playlist {
            name: "Warm up".into(),
            entries: vec![gone],
        }],
    ))
    .unwrap();
    assert_eq!(out.left_out, []);
    assert!(out.sent[0].in_rekordbox && out.sent[0].file_missing);
    assert_eq!(
        attributes(&out.sent[0]),
        [
            ("TrackID", "2"),
            ("Name", "rekordbox's title"),
            ("Rating", "204"),
            ("Location", theirs),
            ("FutureField", "kept"),
        ]
    );
    let read = RekordboxXml::parse(out.xml()).unwrap();
    assert_eq!(read.playlists.playlists()[0].1.entries.len(), 1);
}
