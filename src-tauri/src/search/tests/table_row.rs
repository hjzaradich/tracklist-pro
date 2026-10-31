//! The track table's row is written only when it differs from the stored one
//! (`table_row::write`): for every column of `track_row`, a change must be
//! written and an equal row must not. The columns are read from the table
//! itself, so a column added later fails here until it has a case.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rusqlite::types::Value;
use rusqlite::{Connection, Transaction};

use super::support::Lib;
use crate::search::table_row::{flag, write, Facts, Written};
use crate::send_values::ShownFields;

/// The track the rows are written for.
const TRACK: i64 = 7;

/// Everything `write` is given.
#[derive(Clone)]
struct Inputs {
    fields: ShownFields,
    facts: Facts,
    has_file: bool,
    text_hash: i64,
}

/// A track with something in every column, so each case can change it or
/// blank it.
fn filled() -> Inputs {
    Inputs {
        fields: ShownFields {
            title: "A Title".into(),
            artist: "An Artist".into(),
            genre: "A Genre".into(),
            total_time_s: Some(300),
            stars: Some(3),
            ..ShownFields::default()
        },
        facts: Facts {
            flags: flag::IN_LIBRARY | flag::IN_REKORDBOX,
            format: Some("mp3".into()),
            file_id: Some(4),
            file_name: Some("a file.mp3".into()),
            folder: Some("Music/House".into()),
            duration_ms: Some(290_000),
            bpm: Some(124.0),
            key: Some("8A".into()),
            added_at: Some("2026-10-01T10:00:00Z".into()),
            genre: Some("A Genre".into()),
        },
        has_file: true,
        text_hash: 11,
    }
}

type Change = fn(&mut Inputs);

/// One change to what `write` is given, and the column of `track_row` it
/// changes. Every column has a change to another value, and a column that
/// can be blank has one to blank as well.
const CHANGES: &[(&str, &str, Change)] = &[
    ("flags", "a flag", |i| i.facts.flags ^= flag::IN_CRATE),
    ("flags", "the track having no file", |i| i.has_file = false),
    ("format", "another format", |i| {
        i.facts.format = Some("flac".into())
    }),
    ("format", "no format", |i| i.facts.format = None),
    ("file_id", "another file", |i| i.facts.file_id = Some(5)),
    ("file_id", "no file", |i| i.facts.file_id = None),
    ("title_key", "another title", |i| {
        i.fields.title = "Another Title".into()
    }),
    ("artist_key", "another artist", |i| {
        i.fields.artist = "Another Artist".into()
    }),
    ("artist_key", "no artist", |i| i.fields.artist.clear()),
    ("genre_key", "another genre", |i| {
        i.fields.genre = "Another Genre".into()
    }),
    ("genre_key", "no genre", |i| i.fields.genre.clear()),
    ("file_key", "another file name", |i| {
        i.facts.file_name = Some("another file.mp3".into());
    }),
    ("file_key", "no file name", |i| i.facts.file_name = None),
    ("folder_key", "another folder", |i| {
        i.facts.folder = Some("Music/Techno".into())
    }),
    ("folder_key", "no folder", |i| i.facts.folder = None),
    ("bpm", "another BPM", |i| i.facts.bpm = Some(125.0)),
    ("bpm", "no BPM", |i| i.facts.bpm = None),
    ("key_order", "another key", |i| {
        i.facts.key = Some("9B".into())
    }),
    ("key_order", "no key", |i| i.facts.key = None),
    ("time_ms", "another length from rekordbox", |i| {
        i.fields.total_time_s = Some(301)
    }),
    ("time_ms", "another length from the file", |i| {
        i.fields.total_time_s = None;
        i.facts.duration_ms = Some(291_000);
    }),
    ("time_ms", "no length", |i| {
        i.fields.total_time_s = None;
        i.facts.duration_ms = None;
    }),
    ("rating", "another rating", |i| i.fields.stars = Some(4)),
    ("rating", "no rating", |i| i.fields.stars = None),
    ("added_at", "another date added", |i| {
        i.facts.added_at = Some("2026-10-02T10:00:00Z".into());
    }),
    ("added_at", "no date added", |i| i.facts.added_at = None),
    ("text_hash", "other search text", |i| i.text_hash = 12),
    // The Overview's genre (1cD-R1b): the spelling, and the group it is
    // counted in (the lower case), which only another genre changes.
    ("genre", "another spelling of it", |i| {
        i.facts.genre = Some("a genre".into())
    }),
    ("genre", "no genre", |i| i.facts.genre = None),
    ("genre_group", "another genre", |i| {
        i.facts.genre = Some("Another Genre".into())
    }),
    ("genre_group", "no genre", |i| i.facts.genre = None),
];

/// The columns of `track_row` that are values: all but the track's id.
fn value_columns(conn: &Connection) -> BTreeSet<String> {
    conn.prepare("SELECT name FROM pragma_table_info('track_row') WHERE name <> 'recording_id'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// What `column` of the stored row holds.
fn stored(conn: &Connection, column: &str) -> Value {
    conn.query_row(
        &format!("SELECT {column} FROM track_row WHERE recording_id = ?1"),
        [TRACK],
        |r| r.get(0),
    )
    .unwrap()
}

fn write_inputs(tx: &Transaction<'_>, i: &Inputs) -> Written {
    write(tx, TRACK, &i.fields, &i.facts, i.has_file, i.text_hash).unwrap()
}

/// How many rows of `track_row` `run` inserts, updates or deletes.
fn rows_written(conn: &Connection, run: impl FnOnce()) -> usize {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    let _ = conn.update_hook(Some(
        move |_: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
            if table == "track_row" {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        },
    ));
    run();
    let _ = conn.update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>);
    count.load(Ordering::SeqCst)
}

#[test]
fn every_column_of_the_row_has_a_change_that_is_written_and_an_equal_row_is_not() {
    let lib = Lib::new();
    lib.writer
        .call(|conn| {
            // A column added to `track_row` without a case here fails this.
            let covered: BTreeSet<String> = CHANGES
                .iter()
                .map(|(column, _, _)| (*column).to_owned())
                .collect();
            assert_eq!(
                covered,
                value_columns(conn),
                "every column of track_row needs a change in CHANGES, and every change a column"
            );

            for (column, what, change) in CHANGES {
                conn.execute("DELETE FROM track_row", [])?;
                let tx = conn.transaction()?;
                let base = filled();
                assert_eq!(
                    write_inputs(&tx, &base),
                    Written::Changed { text: true },
                    "a new row"
                );
                let before = stored(&tx, column);

                // The same row again: nothing is written.
                let wrote = rows_written(&tx, || {
                    assert_eq!(
                        write_inputs(&tx, &base),
                        Written::Unchanged,
                        "{column}: {what}"
                    );
                });
                assert_eq!(wrote, 0, "{column}: an equal row is written again");

                // Changed in one way only: written, and the column holds it.
                let mut changed = base.clone();
                change(&mut changed);
                let mut result = None;
                let wrote = rows_written(&tx, || result = Some(write_inputs(&tx, &changed)));
                assert_eq!(
                    result,
                    Some(Written::Changed {
                        text: *column == "text_hash"
                    }),
                    "{column}: {what}"
                );
                assert_eq!(wrote, 1, "{column}: {what} is not written");
                assert_ne!(
                    stored(&tx, column),
                    before,
                    "{column}: {what} changed nothing"
                );

                // And that row, as it is now, is equal to itself.
                assert_eq!(
                    write_inputs(&tx, &changed),
                    Written::Unchanged,
                    "{column}: {what}"
                );
                tx.rollback()?;
            }
            Ok(())
        })
        .unwrap();
}

#[test]
fn a_row_is_also_written_when_only_the_stored_one_differs_in_a_column() {
    // The other direction: the stored row is read back whole, so a column
    // the comparison forgot to read would keep a stale value.
    let lib = Lib::new();
    lib.writer
        .call(|conn| {
            for column in value_columns(conn) {
                let tx = conn.transaction()?;
                let base = filled();
                write_inputs(&tx, &base);
                let right = stored(&tx, &column);
                let wrong = match &right {
                    Value::Integer(n) => Value::Integer(n + 1),
                    Value::Real(x) => Value::Real(x + 1.0),
                    Value::Text(t) => Value::Text(format!("{t} (stale)")),
                    other => {
                        panic!("{column}: the base row holds {other:?}, not a value to change")
                    }
                };
                tx.execute(
                    &format!("UPDATE track_row SET {column} = ?1 WHERE recording_id = ?2"),
                    rusqlite::params![wrong, TRACK],
                )?;
                assert_ne!(stored(&tx, &column), right);
                assert!(
                    matches!(write_inputs(&tx, &base), Written::Changed { .. }),
                    "{column}: a stale stored value is not noticed"
                );
                assert_eq!(stored(&tx, &column), right, "{column} is not put right");
                tx.rollback()?;
            }
            Ok(())
        })
        .unwrap();
}
