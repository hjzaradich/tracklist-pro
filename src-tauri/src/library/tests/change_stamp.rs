//! The Library's size and change stamp (`library_summary`, audit 1cB A1).
//! Synthetic data, like the rest of the tests.

use std::collections::BTreeSet;

use super::*;
use crate::ops::undo_last_via;

impl Lib {
    /// Starts watching the writer, as the app does when it opens it.
    fn stamp(&self) -> LibraryStamp {
        LibraryStamp::install(&self.writer).unwrap()
    }

    fn summary(&self, stamp: &LibraryStamp) -> LibrarySummary {
        let stamp = stamp.clone();
        self.writer
            .call(move |c| stamp::summary(c, &stamp))
            .unwrap()
    }

    /// A Library track for a new track with one present file; returns the
    /// track and its file.
    fn library_track_of(&self, name: &str) -> (i64, i64) {
        let track = self.track(Some(name), None);
        let file = self.file(track, name, "best", true);
        self.promote(track).unwrap();
        (track, file)
    }

    fn exec(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) {
        self.writer.call(move |c| c.execute(sql, params)).unwrap();
    }
}

#[test]
fn the_summary_counts_the_library_tracks() {
    let lib = Lib::new();
    let stamp = lib.stamp();
    assert_eq!(lib.summary(&stamp).tracks, 0);

    lib.library_track_of("a.mp3");
    lib.library_track_of("b.mp3");
    // A track with no Library track isn't counted.
    lib.track(Some("not added"), None);

    let summary = lib.summary(&stamp);
    assert_eq!(summary.tracks, 2);
    assert_eq!(summary.tracks as usize, lib.list().len());
}

#[test]
fn adding_and_undoing_the_add_of_a_library_track_each_move_the_stamp() {
    let lib = Lib::new();
    let stamp = lib.stamp();
    let before = lib.summary(&stamp);

    lib.library_track_of("a.mp3");
    let added = lib.summary(&stamp);
    assert_eq!(added.tracks, 1);
    assert_ne!(added.change_stamp, before.change_stamp);

    undo_last_via(&lib.writer).unwrap();
    let undone = lib.summary(&stamp);
    assert_eq!(undone.tracks, 0);
    assert_ne!(undone.change_stamp, added.change_stamp);
}

#[test]
fn removing_a_library_track_moves_the_stamp() {
    let lib = Lib::new();
    let stamp = lib.stamp();
    let (track, _) = lib.library_track_of("a.mp3");
    let before = lib.summary(&stamp);

    let id = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT id FROM library_track WHERE recording_id = ?1",
                [track],
                |r| r.get(0),
            )
        })
        .unwrap();
    remove(&lib.writer, LibraryTrackId(id)).unwrap();

    let after = lib.summary(&stamp);
    assert_eq!(after.tracks, 0);
    assert_ne!(after.change_stamp, before.change_stamp);
}

/// What a retitle is, wherever the list takes the title from: the track's
/// own title, a tag on its file, rekordbox's entry, the user's own value.
/// Every one leaves the count where it was and the list different.
#[test]
fn a_retitle_with_the_same_count_still_moves_the_stamp() {
    let retitles: [(&str, &'static str); 4] = [
        (
            "the track's own title",
            "UPDATE recording SET title = 'Renamed' WHERE id = 1",
        ),
        (
            "a tag on its file",
            "UPDATE file SET raw_tags = '{\"title\":\"Renamed\"}' WHERE id = 1",
        ),
        (
            "rekordbox's entry",
            // The snapshot is only ever replaced by a fresh read.
            "DELETE FROM rekordbox_track WHERE 1;
             INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, relink_method, relink_probable)
             VALUES ('{\"TrackID\":\"2\",\"Location\":\"file://localhost/E:/Music/2.mp3\",\"Name\":\"Renamed\"}', 'E:/Music/2.mp3',
                     '2026-10-01T10:00:00.000Z', 1, 'path', 0)",
        ),
        (
            "the user's own value",
            "INSERT INTO app_value (library_track_id, field, value) VALUES (1, 'Name', 'Renamed')",
        ),
    ];
    for (what, sql) in retitles {
        let lib = Lib::new();
        let stamp = lib.stamp();
        let (track, file) = lib.library_track_of("a.mp3");
        assert_eq!((track, file), (1, 1), "the statements name these rows");
        lib.rekordbox_match(file, false);
        let before = lib.summary(&stamp);

        lib.writer.call(move |c| c.execute_batch(sql)).unwrap();

        let after = lib.summary(&stamp);
        assert_eq!(after.tracks, before.tracks, "{what}");
        assert_ne!(after.change_stamp, before.change_stamp, "{what}");
    }
}

#[test]
fn relinking_and_a_file_going_missing_move_the_stamp() {
    let lib = Lib::new();
    let stamp = lib.stamp();
    let (track, file) = lib.library_track_of("a.mp3");
    let other = lib.file(track, "b.mp3", "undecided", true);

    let before = lib.summary(&stamp);
    lib.exec(
        "UPDATE library_track SET linked_file_id = ?1 WHERE recording_id = ?2",
        (other, track),
    );
    let relinked = lib.summary(&stamp);
    assert_ne!(relinked.change_stamp, before.change_stamp);

    lib.exec("UPDATE file SET present = 0 WHERE id = ?1", (other,));
    let missing = lib.summary(&stamp);
    assert_ne!(missing.change_stamp, relinked.change_stamp);
    assert_ne!(file, other);
}

#[test]
fn an_open_conflict_and_a_better_file_move_the_stamp() {
    let lib = Lib::new();
    let stamp = lib.stamp();
    let (track, _) = lib.library_track_of("a.mp3");
    lib.file(track, "better.flac", "undecided", true);

    let before = lib.summary(&stamp);
    lib.settle_roles();
    let settled = lib.summary(&stamp);
    // Whatever the roles came to, a row of `file_role`/`better_file` is
    // written through the listed tables or not at all: a stamp that did not
    // move here would be a roles change the list could not show.
    if lib.count("SELECT count(*) FROM better_file") > 0 {
        assert_ne!(settled.change_stamp, before.change_stamp);
    }

    lib.exec(
        "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value)
         VALUES (1, 'Rating', '1', '2', '3')",
        (),
    );
    assert_ne!(lib.summary(&stamp).change_stamp, settled.change_stamp);
}

/// The hooks do not see what SQLite clears in one step: a DELETE with no
/// WHERE clause on a table with no triggers and nothing referring to it
/// (its "truncate optimization") reports no rows. No code deletes from a
/// listed table that way, and this holds it: a delete that must empty a
/// table says `WHERE 1`.
#[test]
fn a_delete_with_no_where_clause_is_not_seen_by_the_hooks_so_no_code_makes_one() {
    // The fact the rule rests on.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY); INSERT INTO t VALUES (1), (2);")
        .unwrap();
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let hook = seen.clone();
    conn.update_hook(Some(
        move |_: rusqlite::hooks::Action, _: &str, _: &str, _: i64| {
            hook.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        },
    ))
    .unwrap();
    conn.execute("DELETE FROM t", ()).unwrap();
    conn.execute_batch("INSERT INTO t VALUES (1); DELETE FROM t WHERE 1;")
        .unwrap();
    // Only the insert and the delete with a WHERE were reported.
    assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 2);

    // The rule: no source file (tests and the migration tests aside) deletes
    // from a listed table with nothing after its name.
    fn visit(dir: &std::path::Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.to_string_lossy().replace('\\', "/");
            if path.is_dir() {
                if !name.ends_with("/tests") && !name.ends_with("/migrations/schema") {
                    visit(&path, found);
                }
            } else if name.ends_with(".rs") && !name.ends_with("tests.rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                let upper = text.to_ascii_uppercase();
                for (at, _) in upper.match_indices("DELETE FROM ") {
                    let rest = &text[at + "DELETE FROM ".len()..];
                    let table: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    let after = rest[table.len()..].trim_start();
                    if LISTED_TABLES.contains(&table.as_str())
                        && !after.to_ascii_uppercase().starts_with("WHERE")
                    {
                        found.push(format!(
                            "{name}: DELETE FROM {table} {}",
                            &after[..after.len().min(20)]
                        ));
                    }
                }
            }
        }
    }
    let mut found = Vec::new();
    visit(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    assert!(
        found.is_empty(),
        "a DELETE with no WHERE clause is not seen by the change stamp; add WHERE 1: {found:?}"
    );
}

#[test]
fn emptying_a_listed_table_moves_the_stamp() {
    let lib = Lib::new();
    let stamp = lib.stamp();
    lib.library_track_of("a.mp3");
    lib.exec(
        "INSERT INTO app_value (library_track_id, field, value) VALUES (1, 'Name', 'Own')",
        (),
    );
    let before = lib.summary(&stamp);

    lib.exec("DELETE FROM app_value", ());
    assert_ne!(lib.summary(&stamp).change_stamp, before.change_stamp);
}

#[test]
fn writes_to_tables_the_list_is_not_made_of_leave_the_stamp_alone() {
    let lib = Lib::new();
    let stamp = lib.stamp();
    lib.library_track_of("a.mp3");
    let before = lib.summary(&stamp);

    // A setting, a crate, a job, the operation log's own bookkeeping.
    lib.exec(
        "INSERT INTO setting (key, value) VALUES ('synthetic', '1')",
        (),
    );
    lib.exec(
        "INSERT INTO crate (kind, name) VALUES ('static', 'Synthetic')",
        (),
    );
    lib.exec("INSERT INTO search_due (recording_id) VALUES (-1)", ());

    assert_eq!(lib.summary(&stamp), before);
}

#[test]
fn a_change_that_is_rolled_back_leaves_the_stamp_alone() {
    let lib = Lib::new();
    let stamp = lib.stamp();
    let (track, _) = lib.library_track_of("a.mp3");
    let before = lib.summary(&stamp);

    lib.writer
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute(
                "UPDATE recording SET title = 'Renamed' WHERE id = ?1",
                [track],
            )?;
            tx.rollback()
        })
        .unwrap();
    assert_eq!(lib.summary(&stamp), before);

    // And the mark a rolled-back change left does not carry into the next
    // commit, which has nothing listed in it.
    lib.exec(
        "INSERT INTO crate (kind, name) VALUES ('static', 'Synthetic')",
        (),
    );
    assert_eq!(lib.summary(&stamp), before);
}

#[test]
fn many_rows_changed_in_one_commit_are_one_move() {
    let lib = Lib::new();
    let stamp = lib.stamp();
    for n in 0..5 {
        lib.library_track_of(&format!("{n}.mp3"));
    }
    let before = lib.summary(&stamp);
    lib.exec("UPDATE recording SET title = 'Same'", ());
    assert_eq!(
        lib.summary(&stamp).change_stamp,
        before.change_stamp.wrapping_add(1)
    );
}

#[test]
fn reading_the_summary_does_not_move_the_stamp() {
    let lib = Lib::new();
    let stamp = lib.stamp();
    lib.library_track_of("a.mp3");
    let first = lib.summary(&stamp);
    assert_eq!(lib.summary(&stamp), first);
    assert_eq!(lib.list().len(), 1);
    assert_eq!(lib.summary(&stamp), first);
}

/// The tables the Library list's own query reads, by the root pages
/// SQLite's plan opens for it.
fn tables_read_by(conn: &Connection, sql: &str) -> BTreeSet<String> {
    let mut plan = conn.prepare(&format!("EXPLAIN {sql}")).unwrap();
    let roots: Vec<i64> = plan
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .filter(|(opcode, _)| opcode == "OpenRead")
        .map(|(_, root)| root)
        .collect();
    roots
        .into_iter()
        .map(|root| {
            conn.query_row(
                "SELECT tbl_name FROM sqlite_master WHERE rootpage = ?1",
                [root],
                |r| r.get(0),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn the_stamp_watches_every_table_the_library_list_reads() {
    let lib = Lib::new();
    let read = lib
        .writer
        .call(|c| Ok(tables_read_by(c, &stored_sql(""))))
        .unwrap();
    assert!(
        read.contains("library_track"),
        "the plan was read: {read:?}"
    );
    let missing: Vec<&String> = read
        .iter()
        .filter(|table| !LISTED_TABLES.contains(&table.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "the list reads {missing:?}; add to LISTED_TABLES"
    );
}
