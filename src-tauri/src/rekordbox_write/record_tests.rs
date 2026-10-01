//! What a send records, over a migrated database in a temp dir.

use super::*;
use crate::db::{DbError, Writer};

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
        Lib { _dir: dir, writer }
    }

    /// A Library track. Its file doesn't matter here.
    fn library_track(&self) -> LibraryTrackId {
        self.writer
            .call(|c| {
                c.execute("INSERT INTO recording DEFAULT VALUES", [])?;
                c.execute(
                    "INSERT INTO library_track (recording_id, kind, source_status)
                     VALUES (?1, 'linked', 'missing')",
                    [c.last_insert_rowid()],
                )?;
                Ok(LibraryTrackId(c.last_insert_rowid()))
            })
            .unwrap()
    }

    fn record(&self, sent: &[SentTrack]) -> Result<(), DbError> {
        let sent = sent.to_vec();
        self.writer.call(move |c| record_send(c, &sent))
    }

    /// Every `sync_base` row of a track: field and value, by field.
    fn base(&self, track: LibraryTrackId) -> Vec<(String, String)> {
        self.writer
            .call(move |c| {
                c.prepare(
                    "SELECT field, value FROM sync_base WHERE library_track_id = ?1
                     ORDER BY field",
                )?
                .query_map([track.0], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
            })
            .unwrap()
    }

    /// A track's `last_sent_location` and whether `last_exported_at` is set.
    fn mark(&self, track: LibraryTrackId) -> (Option<String>, bool) {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT last_sent_location, last_exported_at IS NOT NULL
                     FROM library_track WHERE id = ?1",
                    [track.0],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .unwrap()
    }

    fn execute(&self, sql: &'static str) -> Result<usize, DbError> {
        self.writer.call(move |c| c.execute(sql, []))
    }
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
        .collect()
}

/// Builds a send of these tracks, each in one crate, and returns what was
/// sent.
fn sent(tracks: Vec<TrackInput>) -> Vec<SentTrack> {
    let entries = tracks.iter().map(|t| t.library_track).collect();
    let out = build(&SendInput {
        tracks,
        crates: vec![Node::Playlist {
            name: "C".into(),
            entries,
        }],
        ..SendInput::default()
    })
    .unwrap();
    assert_eq!(out.left_out, []);
    out.sent
}

fn known(id: LibraryTrackId, attributes: &[(&str, &str)]) -> TrackInput {
    TrackInput {
        library_track: id,
        values: Values::Ready {
            in_rekordbox: true,
            attributes: pairs(attributes),
            rekordbox_holds_other_file: None,
        },
    }
}

fn new(id: LibraryTrackId, attributes: &[(&str, &str)]) -> TrackInput {
    TrackInput {
        library_track: id,
        values: Values::Ready {
            in_rekordbox: false,
            attributes: pairs(attributes),
            rekordbox_holds_other_file: None,
        },
    }
}

#[test]
fn sync_base_holds_every_field_sent_under_its_xml_attribute_name() {
    let lib = Lib::new();
    let a = lib.library_track();
    let b = lib.library_track();
    let sent = sent(vec![
        known(
            a,
            &[
                ("TrackID", "40"),
                ("Name", "Kit & Kin's\ttabbed"),
                ("Artist", ""),
                ("AverageBpm", "128.00"),
                ("Rating", "204"),
                ("Location", "file://localhost/C:/Kit/a%20b%20#1.mp3"),
                ("Tonality", "4A"),
                ("FutureField", "kept"),
            ],
        ),
        new(b, &[("Name", "New"), ("Location", r"C:\New\n & m.flac")]),
    ]);
    lib.record(&sent).unwrap();

    // Exactly what the file carries: values as written (unescaped), the
    // Location in its encoded form; no TrackID, no analysis field.
    assert_eq!(
        lib.base(a),
        pairs(&[
            ("Artist", ""),
            ("FutureField", "kept"),
            ("Location", "file://localhost/C:/Kit/a%20b%20%231.mp3"),
            ("Name", "Kit & Kin's\ttabbed"),
            ("Rating", "204"),
        ])
    );
    assert_eq!(
        lib.base(b),
        pairs(&[
            ("Location", "file://localhost/C:/New/n%20%26%20m.flac"),
            ("Name", "New"),
        ])
    );
    // One row per field written, and the same fields.
    for track in &sent {
        let fields: Vec<(String, String)> = {
            let mut f: Vec<_> = track
                .fields()
                .map(|(n, v)| (n.to_owned(), v.to_owned()))
                .collect();
            f.sort();
            f
        };
        assert_eq!(lib.base(track.library_track), fields);
        assert_eq!(fields.len(), track.attributes.len() - 1, "all but TrackID");
    }
    // The send is marked on each track, in the same step.
    assert_eq!(
        lib.mark(a),
        (
            Some("file://localhost/C:/Kit/a%20b%20%231.mp3".into()),
            true
        )
    );
    assert_eq!(
        lib.mark(b),
        (
            Some("file://localhost/C:/New/n%20%26%20m.flac".into()),
            true
        )
    );
}

#[test]
fn a_second_send_replaces_the_base_values_and_never_duplicates_them() {
    let lib = Lib::new();
    let a = lib.library_track();
    let other = lib.library_track();
    let location = ("Location", "file://localhost/C:/Kit/a.mp3");
    lib.record(&sent(vec![
        known(
            a,
            &[
                ("TrackID", "40"),
                ("Name", "First"),
                ("Rating", "51"),
                location,
            ],
        ),
        new(other, &[("Name", "Other"), ("Location", r"C:\New\o.mp3")]),
    ]))
    .unwrap();
    let other_before = lib.base(other);

    // Rating changed, Name is no longer sent, Genre is new; `other` isn't
    // in this send.
    lib.record(&sent(vec![known(
        a,
        &[
            ("TrackID", "41"),
            ("Rating", "255"),
            ("Genre", "TLP Genre"),
            location,
        ],
    )]))
    .unwrap();
    assert_eq!(
        lib.base(a),
        pairs(&[
            ("Genre", "TLP Genre"),
            ("Location", "file://localhost/C:/Kit/a.mp3"),
            ("Rating", "255"),
        ])
    );
    assert_eq!(lib.base(other), other_before);

    // The same send again changes nothing.
    let again = sent(vec![known(
        a,
        &[
            ("TrackID", "41"),
            ("Rating", "255"),
            ("Genre", "TLP Genre"),
            location,
        ],
    )]);
    lib.record(&again).unwrap();
    assert_eq!(lib.base(a).len(), 3);
}

#[test]
fn a_send_that_cannot_be_recorded_whole_records_nothing() {
    let lib = Lib::new();
    let a = lib.library_track();
    let location = ("Location", "file://localhost/C:/Kit/a.mp3");
    lib.record(&sent(vec![known(
        a,
        &[("TrackID", "40"), ("Name", "First"), location],
    )]))
    .unwrap();
    let before = (lib.base(a), lib.mark(a));

    // The second track of this send isn't a Library track.
    let failed = lib.record(&sent(vec![
        known(a, &[("TrackID", "40"), ("Name", "Second"), location]),
        new(
            LibraryTrackId(999),
            &[("Name", "Ghost"), ("Location", r"C:\New\g.mp3")],
        ),
    ]));
    assert!(failed.is_err());
    assert_eq!((lib.base(a), lib.mark(a)), before);
    assert_eq!(lib.base(LibraryTrackId(999)), []);
}

#[test]
fn an_empty_send_records_nothing() {
    let lib = Lib::new();
    let a = lib.library_track();
    lib.record(&[]).unwrap();
    assert_eq!(lib.base(a), []);
    assert_eq!(lib.mark(a), (None, false));
}
