//! Vocabulary tests, over a migrated database in a temp dir. Everything is
//! synthetic: made-up tracks and tags, no files.

use rusqlite::types::Value;
use rusqlite::Connection;

use super::import::{import_on, TagsImported};
use super::*;
use crate::db::Writer;
use crate::ops::{undo_last_via, UndoOutcome};

struct Lib {
    writer: Writer,
    // Last: fields drop in order, so the connections close before the folder goes.
    _dir: tempfile::TempDir,
}

/// Every row of a query, as text, for comparing whole states.
fn dump(conn: &Connection, sql: &str) -> Vec<Vec<String>> {
    let mut stmt = conn.prepare(sql).unwrap();
    let columns = stmt.column_count();
    stmt.query_map([], |r| {
        (0..columns)
            .map(|i| r.get::<_, Value>(i).map(|v| format!("{v:?}")))
            .collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
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

    fn run(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, params)?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    /// A track, in the Library when `library`: its id.
    fn track(&self, title: &'static str, library: bool) -> i64 {
        let recording = self.run(
            "INSERT INTO recording (title, artist) VALUES (?1, 'Kit')",
            (title,),
        );
        if library {
            self.run(
                "INSERT INTO library_track (recording_id, source_status) VALUES (?1, 'missing')",
                (recording,),
            );
        }
        recording
    }

    /// One row of the last rekordbox read, with these My Tags, matched to
    /// `recording` (or to none). `my_tags` is JSON: rows are made the way a
    /// read makes them, so a value that isn't a name can be put in.
    fn rekordbox(&self, id: i64, my_tags: &'static str, recording: Option<i64>) {
        self.rekordbox_at(id, my_tags, recording, "2026-01-01T00:00:00.000Z");
    }

    /// [`Lib::rekordbox`], in the read made at `read_at`.
    fn rekordbox_at(
        &self,
        id: i64,
        my_tags: &'static str,
        recording: Option<i64>,
        read_at: &'static str,
    ) {
        self.run(
            "INSERT INTO rekordbox_track (attributes, location_key, my_tags, read_at, recording_id)
             VALUES (json_object('TrackID', CAST(?1 AS TEXT), 'Location', 'file://localhost/E:/x.mp3'),
                     'e:/x' || ?1, ?2, ?4, ?3)",
            (id, my_tags, recording, read_at),
        );
    }

    fn forget_rekordbox(&self) {
        self.run("DELETE FROM rekordbox_track", ());
    }

    fn group(&self, name: &'static str) -> TagGroupId {
        self.writer
            .call(move |c| Ok(create_group_on(c, name)))
            .unwrap()
            .unwrap()
            .unwrap()
    }

    fn tag(&self, group: TagGroupId, name: &'static str) -> TagId {
        self.writer
            .call(move |c| Ok(create_tag_on(c, group, name)))
            .unwrap()
            .unwrap()
            .unwrap()
    }

    fn apply(&self, tag: TagId, recordings: &[i64]) -> TagsChanged {
        let recordings = recordings.to_vec();
        self.writer
            .call(move |c| Ok(apply_on(c, tag, &recordings)))
            .unwrap()
            .unwrap()
            .unwrap()
    }

    fn import(&self) -> TagsImported {
        self.writer
            .call(|c| Ok(import_on(c, "From rekordbox")))
            .unwrap()
            .unwrap()
            .unwrap()
    }

    fn undo(&self) {
        assert!(matches!(
            undo_last_via(&self.writer).unwrap(),
            UndoOutcome::Undone { .. }
        ));
    }

    fn listed(&self) -> Vec<TagGroup> {
        self.writer.call(|c| list(c)).unwrap()
    }

    /// The tags' names and the tracks they are on.
    fn state(&self) -> Vec<Vec<Vec<String>>> {
        self.writer
            .call(|c| {
                Ok(["tag_group", "tag", "tag_applied"]
                    .map(|t| dump(c, &format!("SELECT * FROM {t} ORDER BY id")))
                    .to_vec())
            })
            .unwrap()
    }

    fn names(&self) -> Vec<String> {
        self.listed()
            .into_iter()
            .flat_map(|g| g.tags)
            .map(|t| t.name)
            .collect()
    }

    fn tag_named(&self, name: &str) -> Tag {
        self.listed()
            .into_iter()
            .flat_map(|g| g.tags)
            .find(|t| t.name == name)
            .unwrap()
    }
}

#[test]
fn names_are_trimmed_and_must_be_free_and_sendable() {
    let lib = Lib::new();
    let vibe = lib.group("  Vibe ");
    assert_eq!(lib.listed()[0].name, "Vibe");
    let warm = lib.tag(vibe, " Warm ");
    assert_eq!(lib.names(), ["Warm"]);

    let refused = |name: &'static str| {
        lib.writer
            .call(move |c| Ok(create_tag_on(c, vibe, name)))
            .unwrap()
            .unwrap()
            .unwrap_err()
    };
    assert!(matches!(refused("   "), VocabError::NameEmpty));
    // Equal by the writer's sibling rule, whatever the group.
    assert!(matches!(refused("WARM"), VocabError::NameTaken { existing } if existing == "Warm"));
    let other = lib.group("Role");
    let taken = lib
        .writer
        .call(move |c| Ok(create_tag_on(c, other, "warm")))
        .unwrap()
        .unwrap();
    assert!(matches!(taken, Err(VocabError::NameTaken { .. })));
    // A character XML can't carry.
    assert!(matches!(refused("a\u{1}b"), VocabError::NameUnsendable));
    let missing = lib
        .writer
        .call(|c| Ok(create_tag_on(c, TagGroupId(99), "x")))
        .unwrap()
        .unwrap();
    assert!(matches!(missing, Err(VocabError::GroupNotFound)));
    let again = lib
        .writer
        .call(|c| Ok(create_group_on(c, "vibe")))
        .unwrap()
        .unwrap();
    assert!(matches!(again, Err(VocabError::NameTaken { .. })));
    // The refusals left nothing behind.
    assert_eq!(lib.names(), ["Warm"]);
    assert_eq!(lib.listed().len(), 2);
    assert_eq!(warm, lib.tag_named("Warm").id);
}

#[test]
fn rename_and_colour_are_undoable_and_checked() {
    let lib = Lib::new();
    let group = lib.group("Vibe");
    let tag = lib.tag(group, "Warm");
    let before = lib.state();

    let item = VocabItem::Tag { id: tag };
    let renamed = lib
        .writer
        .call(move |c| Ok(rename_on(c, item, " Hot ")))
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(renamed.is_some());
    assert_eq!(lib.names(), ["Hot"]);
    // The name it has already records nothing.
    let same = lib
        .writer
        .call(move |c| Ok(rename_on(c, item, "Hot")))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(same, None);
    lib.undo();
    assert_eq!(lib.state(), before);

    for bad in ["red", "#12345", "#gggggg", "#1234567"] {
        let refused = lib
            .writer
            .call(move |c| Ok(colour_on(c, item, Some(bad))))
            .unwrap()
            .unwrap();
        assert!(matches!(refused, Err(VocabError::ColourInvalid)), "{bad}");
    }
    let done = lib
        .writer
        .call(move |c| Ok(colour_on(c, item, Some("#E05A47"))))
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(done.is_some());
    assert_eq!(lib.tag_named("Warm").colour.as_deref(), Some("#e05a47"));
    let group_item = VocabItem::Group { id: group };
    lib.writer
        .call(move |c| Ok(colour_on(c, group_item, Some("#3b82f6"))))
        .unwrap()
        .unwrap()
        .unwrap();
    lib.undo();
    lib.undo();
    assert_eq!(lib.state(), before);
    let gone = lib
        .writer
        .call(|c| Ok(rename_on(c, VocabItem::Tag { id: TagId(99) }, "x")))
        .unwrap()
        .unwrap();
    assert!(matches!(gone, Err(VocabError::TagNotFound)));
}

#[test]
fn applying_and_removing_skip_what_is_already_so_and_undo_restores_each() {
    let lib = Lib::new();
    let tag = lib.tag(lib.group("Vibe"), "Warm");
    let (a, b) = (lib.track("One", true), lib.track("Two", false));
    let empty = lib.state();

    let first = lib.apply(tag, &[a, a, 999]);
    assert_eq!((first.changed, first.skipped), (1, 2));
    let with_a = lib.state();
    let second = lib.apply(tag, &[a, b]);
    assert_eq!((second.changed, second.skipped), (1, 1));
    // Nothing to do records nothing.
    let nothing = lib.apply(tag, &[a, b]);
    assert_eq!((nothing.changed, nothing.operation_id), (0, None));
    assert_eq!(lib.tag_named("Warm").track_count, 2);
    assert_eq!(lib.writer.call(move |c| tags_of(c, b)).unwrap(), [tag]);

    let removed = lib
        .writer
        .call(move |c| Ok(remove_on(c, tag, &[a, 999])))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!((removed.changed, removed.skipped), (1, 1));
    lib.undo();
    lib.undo();
    assert_eq!(lib.state(), with_a);
    lib.undo();
    assert_eq!(lib.state(), empty);
    let missing = lib
        .writer
        .call(move |c| Ok(apply_on(c, TagId(99), &[a])))
        .unwrap()
        .unwrap();
    assert!(matches!(missing, Err(VocabError::TagNotFound)));
}

#[test]
fn the_import_copies_my_tags_one_to_one_as_one_undoable_operation() {
    let lib = Lib::new();
    let (a, b) = (lib.track("One", true), lib.track("Two", false));
    // The rows hold a tag twice, a number, a blank and a tag on a track that
    // matches none: none of it stops the import.
    lib.rekordbox(1, r#"["Sunrise","Dusk"]"#, Some(a));
    lib.rekordbox(2, r#"["Sunrise", 7, "", "  "]"#, Some(b));
    lib.rekordbox(3, r#"["Sunrise","Midnight"]"#, None);
    let before = lib.state();

    let done = lib.import();
    assert_eq!(
        (done.added, done.applied, done.skipped, done.gone, done.back),
        (3, 3, 0, 0, 0)
    );
    assert!(done.operation_id.is_some());
    let groups = lib.listed();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "From rekordbox");
    let names: Vec<&str> = groups[0].tags.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Dusk", "Midnight", "Sunrise"]);
    let sunrise = lib.tag_named("Sunrise");
    assert_eq!(sunrise.rekordbox_name.as_deref(), Some("Sunrise"));
    assert!(!sunrise.gone_from_rekordbox);
    assert_eq!(sunrise.track_count, 2);
    assert_eq!(lib.tag_named("Midnight").track_count, 0);

    // Again: nothing changed, nothing recorded.
    let again = lib.import();
    assert_eq!(
        (again.added, again.applied, again.operation_id),
        (0, 0, None)
    );

    // One Undo takes the whole import back.
    lib.undo();
    assert_eq!(lib.state(), before);
}

#[test]
fn a_my_tag_that_disappears_stays_flagged_and_a_removed_tag_is_not_put_back() {
    let lib = Lib::new();
    let a = lib.track("One", true);
    lib.rekordbox(1, r#"["Sunrise","Dusk"]"#, Some(a));
    lib.import();
    let dusk = lib.tag_named("Dusk").id;
    lib.writer
        .call(move |c| Ok(remove_on(c, dusk, &[a])))
        .unwrap()
        .unwrap()
        .unwrap();

    // rekordbox no longer has Dusk.
    lib.forget_rekordbox();
    lib.rekordbox(1, r#"["Sunrise"]"#, Some(a));
    let done = lib.import();
    assert_eq!((done.added, done.gone), (0, 1));
    assert!(lib.tag_named("Dusk").gone_from_rekordbox);
    assert!(!lib.tag_named("Sunrise").gone_from_rekordbox);
    let flagged = lib.state();

    // Back again: unflagged, and the track the user took it off stays off.
    lib.forget_rekordbox();
    lib.rekordbox(1, r#"["Sunrise","Dusk"]"#, Some(a));
    let back = lib.import();
    assert_eq!((back.back, back.applied), (1, 0));
    assert!(!lib.tag_named("Dusk").gone_from_rekordbox);
    assert_eq!(lib.tag_named("Dusk").track_count, 0);
    lib.undo();
    assert_eq!(lib.state(), flagged);

    // No read at all says nothing about what rekordbox has: nothing is flagged.
    lib.forget_rekordbox();
    let none = lib.import();
    assert_eq!((none.gone, none.operation_id), (0, None));
    assert_eq!(lib.names().len(), 2);
}

#[test]
fn the_import_links_a_same_named_tag_and_skips_names_that_clash_or_cannot_be_sent() {
    let lib = Lib::new();
    let a = lib.track("One", true);
    let mine = lib.tag(lib.group("Mine"), "sunrise");
    lib.rekordbox(
        1,
        r#"["Sunrise","SUNRISE","bad\u0001name","Fine"]"#,
        Some(a),
    );
    let done = lib.import();
    // "sunrise" is linked (to "SUNRISE", the first in order), "Sunrise" is the same name, the control
    // character can't be sent, "Fine" is new (in a group of its own name).
    assert_eq!((done.added, done.skipped, done.applied), (2, 2, 2));
    let linked = lib.tag_named("sunrise");
    assert_eq!(
        (linked.id, linked.rekordbox_name.as_deref()),
        (mine, Some("SUNRISE"))
    );
    let groups = lib.listed();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[1].name, "From rekordbox");
    assert_eq!(groups[1].tags.len(), 1);
}

#[test]
fn a_blank_group_name_is_refused_before_anything_is_written() {
    let lib = Lib::new();
    let a = lib.track("One", true);
    lib.rekordbox(1, r#"["Sunrise"]"#, Some(a));
    let refused = lib
        .writer
        .call(|c| Ok(import_on(c, "  ")))
        .unwrap()
        .unwrap();
    assert!(matches!(refused, Err(VocabError::NameEmpty)));
    assert!(lib.listed().is_empty());
}

#[test]
fn a_send_writes_one_playlist_per_tag_under_playlists_tags_for_library_tracks_only() {
    let lib = Lib::new();
    let group = lib.group("Vibe");
    let (warm, quiet) = (lib.tag(group, "Warm"), lib.tag(group, "Quiet"));
    let (a, b, outside) = (
        lib.track("One", true),
        lib.track("Two", true),
        lib.track("Three", false),
    );
    lib.apply(warm, &[b, outside, a]);
    // A tag with no Library track has no playlist.
    lib.apply(quiet, &[outside]);

    let tree = lib.writer.call(|c| crate::send::tag_playlists(c)).unwrap();
    let ids: Vec<i64> = lib
        .writer
        .call(|c| {
            c.prepare("SELECT id FROM library_track ORDER BY recording_id")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap();
    let Node::Folder { name, children } = &tree[0] else {
        panic!("{tree:?}")
    };
    assert_eq!((tree.len(), name.as_str(), children.len()), (1, "Tags", 1));
    // In the order they were tagged: b first.
    assert_eq!(
        children[0],
        Node::Playlist {
            name: "Warm".into(),
            entries: vec![
                crate::library::LibraryTrackId(ids[1]),
                crate::library::LibraryTrackId(ids[0])
            ],
        }
    );
    // The writer takes it, and writes exactly that.
    let input = SendInput {
        playlists: tree,
        ..SendInput::default()
    };
    assert!(build(&input).is_ok());

    // No tagged Library track: no folder.
    lib.writer
        .call(move |c| Ok(remove_on(c, warm, &[a, b, outside])))
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(lib
        .writer
        .call(|c| crate::send::tag_playlists(c))
        .unwrap()
        .is_empty());
}

#[test]
fn the_import_reads_only_the_newest_rekordbox_read() {
    let lib = Lib::new();
    let (a, b) = (lib.track("One", true), lib.track("Two", true));
    // A row kept from an older read has a My Tag the newest read lacks.
    lib.rekordbox_at(1, r#"["Old"]"#, Some(a), "2026-01-01T00:00:00.000Z");
    lib.rekordbox_at(2, r#"["Sunrise"]"#, Some(b), "2026-02-01T00:00:00.000Z");
    let done = lib.import();
    assert_eq!((done.added, done.applied), (1, 1));
    assert_eq!(lib.names(), ["Sunrise"]);

    // One imported while the older read was the newest is flagged gone once a
    // newer read no longer has it, though the older row is still kept.
    let lib = Lib::new();
    let (a, b) = (lib.track("One", true), lib.track("Two", true));
    lib.rekordbox_at(
        1,
        r#"["Old","Sunrise"]"#,
        Some(a),
        "2026-01-01T00:00:00.000Z",
    );
    assert_eq!(lib.import().added, 2);
    lib.rekordbox_at(2, r#"["Sunrise"]"#, Some(b), "2026-02-01T00:00:00.000Z");
    let done = lib.import();
    assert_eq!((done.added, done.gone), (0, 1));
    assert!(lib.tag_named("Old").gone_from_rekordbox);
    assert!(!lib.tag_named("Sunrise").gone_from_rekordbox);

    // With only one read, its rows are the newest.
    let lib = Lib::new();
    let a = lib.track("One", true);
    lib.rekordbox_at(1, r#"["Old"]"#, Some(a), "2026-01-01T00:00:00.000Z");
    assert_eq!(lib.import().added, 1);
}

#[test]
fn a_send_with_tags_keeps_comments_records_the_tag_playlists_and_lists_a_dropped_one_as_stale() {
    use crate::after_send::RekordboxTree;
    use crate::after_send::{app_tree, load_recorded, save_tree, stale_playlists, StaleKind};
    use crate::library::LibraryTrackId;
    use crate::rekordbox_write::{record_send, TrackInput, Values};

    let lib = Lib::new();
    let group = lib.group("Vibe");
    let (warm, quiet) = (lib.tag(group, "Warm"), lib.tag(group, "Quiet"));
    let (a, b) = (lib.track("One", true), lib.track("Two", true));
    lib.apply(warm, &[a]);
    lib.apply(quiet, &[b]);
    let ids: Vec<LibraryTrackId> = lib
        .writer
        .call(|c| {
            c.prepare("SELECT id FROM library_track ORDER BY recording_id")?
                .query_map([], |r| r.get(0).map(LibraryTrackId))?
                .collect()
        })
        .unwrap();

    // The send: the tracks' values as they are (a Comments value that ends in
    // a My Tags block among them), and the tags' playlists.
    let comments = "note /* Warm / Quiet */ x /* TLP */";
    let tracks: Vec<TrackInput> = ids
        .iter()
        .enumerate()
        .map(|(at, id)| TrackInput {
            library_track: *id,
            values: Values::Ready {
                in_rekordbox: false,
                attributes: vec![
                    ("Name".into(), format!("Track {at}")),
                    ("Comments".into(), comments.into()),
                    ("Location".into(), format!(r"C:\Music\{at}.mp3")),
                ],
                rekordbox_holds_other_file: None,
                file_missing: false,
            },
        })
        .collect();
    let playlists = lib.writer.call(|c| crate::send::tag_playlists(c)).unwrap();
    let input = SendInput {
        tracks,
        playlists,
        ..SendInput::default()
    };
    let built = build(&input).unwrap();
    let xml = String::from_utf8(built.xml().to_vec()).unwrap();
    // Comments goes out as it was, twice, and no My Tags block is added.
    assert_eq!(xml.matches(&format!("Comments=\"{comments}\"")).count(), 2);
    assert_eq!(xml.matches("/*").count(), 4);
    let written: Vec<Vec<String>> = built.paths().iter().map(|p| p.path.clone()).collect();
    let path = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
    assert!(written.contains(&path(&["Playlists", "Tags"])));
    assert!(written.contains(&path(&["Playlists", "Tags", "Warm"])));
    assert!(written.contains(&path(&["Playlists", "Tags", "Quiet"])));

    // Recorded like a crate, so the Tags playlists can be told from the
    // user's own.
    let (sent, paths) = (built.sent.clone(), built.paths().to_vec());
    lib.writer
        .call(move |c| record_send(c, &sent, &paths))
        .unwrap();
    // rekordbox imports the file and a read shows what it holds.
    let read = lib
        .writer
        .call(|c| {
            Ok(RekordboxTree::from_nodes(
                &crate::send::crate_tree(c)?,
                &crate::send::tag_playlists(c)?,
            ))
        })
        .unwrap();
    let shown = read.clone();
    lib.writer.call(move |c| save_tree(c, &shown)).unwrap();
    let recorded = lib.writer.call(|c| load_recorded(c)).unwrap();
    let app = lib.writer.call(|c| app_tree(c)).unwrap();
    assert!(stale_playlists(&read, &app, &recorded).is_empty());

    // Quiet loses its last Library track: rekordbox still has the playlist,
    // the app no longer sends it.
    lib.writer
        .call(move |c| Ok(remove_on(c, quiet, &[b])))
        .unwrap()
        .unwrap()
        .unwrap();
    let app = lib.writer.call(|c| app_tree(c)).unwrap();
    let stale = stale_playlists(&read, &app, &recorded);
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].path, path(&["Playlists", "Tags", "Quiet"]));
    assert_eq!(stale[0].kind, StaleKind::Playlist);
}
