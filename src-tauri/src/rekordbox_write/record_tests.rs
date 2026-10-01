//! What a send records, over a migrated database in a temp dir.

use super::*;
use crate::db::{DbError, Writer};
use crate::ops::{self, UndoOutcome};

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

    /// The row id of a track's base value for `field`.
    fn row(&self, track: LibraryTrackId, field: &'static str) -> i64 {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT id FROM sync_base WHERE library_track_id = ?1 AND field = ?2",
                    rusqlite::params![track.0, field],
                    |r| r.get(0),
                )
            })
            .unwrap()
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
            file_missing: false,
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
            file_missing: false,
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
    let rating_row = lib.row(a, "Rating");

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
    // A field sent again is the same row with the new value.
    assert_eq!(lib.row(a, "Rating"), rating_row);

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
fn a_track_that_is_not_in_the_library_stops_the_record_by_itself() {
    // Even with nothing else to refuse its rows (foreign keys off), a
    // sent track with no Library track to mark fails the whole record.
    let lib = Lib::new();
    let a = lib.library_track();
    let both = sent(vec![
        new(a, &[("Name", "Real"), ("Location", r"C:\New\a.mp3")]),
        new(
            LibraryTrackId(999),
            &[("Name", "Ghost"), ("Location", r"C:\New\g.mp3")],
        ),
    ]);
    let failed = lib.writer.call(move |c| {
        c.pragma_update(None, "foreign_keys", false)?;
        let result = record_send(c, &both);
        c.pragma_update(None, "foreign_keys", true)?;
        Ok(matches!(
            result,
            Err(rusqlite::Error::StatementChangedRows(0))
        ))
    });
    assert!(failed.unwrap(), "no Library track was there to mark");
    assert_eq!(lib.base(a), []);
    assert_eq!(lib.base(LibraryTrackId(999)), []);
    assert_eq!(lib.mark(a), (None, false));
}

#[test]
fn an_analysis_field_is_never_recorded_whatever_the_caller_hands_in() {
    let lib = Lib::new();
    let a = lib.library_track();
    let location = "file://localhost/C:/Kit/a.mp3";
    // A base from an earlier send, to show the drop of unsent fields
    // doesn't depend on the analysis names either.
    lib.record(&sent(vec![known(
        a,
        &[("TrackID", "40"), ("Name", "First"), ("Location", location)],
    )]))
    .unwrap();

    // Hand-built: `build` would never produce these.
    let direct = SentTrack {
        library_track: a,
        in_rekordbox: true,
        track_id: 40,
        attributes: pairs(&[
            ("TrackID", "40"),
            ("AverageBpm", "128.00"),
            ("Name", "Second"),
            ("Tonality", "4A"),
            ("TEMPO", "x"),
            ("POSITION_MARK", "y"),
            ("Location", location),
        ]),
        rekordbox_holds_other_file: None,
        file_missing: false,
    };
    lib.record(&[direct]).unwrap();
    assert_eq!(
        lib.base(a),
        pairs(&[("Location", location), ("Name", "Second")])
    );
}

#[test]
fn a_track_left_out_of_a_send_keeps_its_earlier_base() {
    let lib = Lib::new();
    let a = lib.library_track();
    let b = lib.library_track();
    lib.record(&sent(vec![
        known(
            a,
            &[
                ("TrackID", "40"),
                ("Name", "A"),
                ("Location", "file://localhost/C:/Kit/a.mp3"),
            ],
        ),
        known(
            b,
            &[
                ("TrackID", "41"),
                ("Name", "B"),
                ("Location", "file://localhost/C:/Kit/b.mp3"),
            ],
        ),
    ]))
    .unwrap();
    let (base_before, mark_before) = (lib.base(b), lib.mark(b));

    // The next send can't carry `b` (a value XML can't hold).
    let out = build(&SendInput {
        tracks: vec![
            known(
                a,
                &[
                    ("TrackID", "40"),
                    ("Name", "A2"),
                    ("Location", "file://localhost/C:/Kit/a.mp3"),
                ],
            ),
            known(
                b,
                &[
                    ("TrackID", "41"),
                    ("Name", "bell\u{7}"),
                    ("Location", "file://localhost/C:/Kit/b.mp3"),
                ],
            ),
        ],
        crates: vec![Node::Playlist {
            name: "C".into(),
            entries: vec![a, b],
        }],
        ..SendInput::default()
    })
    .unwrap();
    assert_eq!(out.left_out.len(), 1);
    lib.record(&out.sent).unwrap();
    assert_eq!(lib.base(b), base_before);
    assert_eq!(lib.mark(b), mark_before);
    assert!(lib.base(a).contains(&("Name".to_owned(), "A2".to_owned())));
}

#[test]
fn undoing_a_removal_is_refused_once_a_later_send_took_its_base_rows() {
    // A send isn't in the operation log. Removing a track deletes its
    // bases (undoably); a later send's new rows can take their freed row
    // ids, and the undo is then refused, changing nothing.
    let lib = Lib::new();
    let removed = lib.library_track();
    // Made before the removal, so only the base rows' ids are reused.
    let other = lib.library_track();
    lib.record(&sent(vec![new(
        removed,
        &[("Name", "T"), ("Location", r"C:\New\t.mp3")],
    )]))
    .unwrap();
    crate::library::remove(&lib.writer, removed).unwrap();
    assert_eq!(lib.base(removed), []);

    lib.record(&sent(vec![new(
        other,
        &[("Name", "X"), ("Location", r"C:\New\x.mp3")],
    )]))
    .unwrap();
    let other_before = (lib.base(other), lib.mark(other));

    match ops::undo_last_via(&lib.writer).unwrap() {
        UndoOutcome::Refused { conflicts, .. } => {
            assert!(!conflicts.is_empty());
            assert!(
                conflicts.iter().all(|c| c.entity == "sync_base"),
                "{conflicts:?}"
            );
        }
        other => panic!("expected the undo to be refused, got {other:?}"),
    }
    // Nothing changed: the track stays removed, the later send's record
    // is whole.
    assert_eq!(lib.base(removed), []);
    assert_eq!((lib.base(other), lib.mark(other)), other_before);
    let still_there: i64 = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT count(*) FROM library_track WHERE id = ?1",
                [removed.0],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(still_there, 0);
}

#[test]
fn an_empty_send_records_nothing() {
    let lib = Lib::new();
    let a = lib.library_track();
    lib.record(&[]).unwrap();
    assert_eq!(lib.base(a), []);
    assert_eq!(lib.mark(a), (None, false));
}
