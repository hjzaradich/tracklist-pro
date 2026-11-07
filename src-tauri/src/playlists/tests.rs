//! Playlist tests (3.5), over a migrated database in a temp dir. Everything
//! is synthetic: made-up tracks, files and rekordbox exports.

use rusqlite::types::Value;
use rusqlite::Connection;

use super::import::{copy_on, rekordbox_playlists, CopiedFromRekordbox};
use super::*;
use crate::db::Writer;
use crate::ops::{undo_last_via, UndoOutcome};
use crate::rekordbox::store::{replace_snapshot, SnapshotRows};
use crate::rekordbox::{EntryTarget, RekordboxXml};
use crate::rekordbox_write::{build, SendInput, TrackInput, Values};

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
        writer
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES ('serial=1A2B3C4D', 'external', 'E:\\');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (1, 'Music', 'Music');",
                )
            })
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

    /// A track with one file named after it; a Library track too when
    /// `library`. Returns (file, Library track or 0).
    fn track(&self, name: &'static str, library: bool) -> (i64, LibraryTrackId) {
        let recording = self.run("INSERT INTO recording (title) VALUES (?1)", [name]);
        let file = self.run(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
             VALUES (1, ?1 || '.mp3', ?1 || '.mp3', 1)",
            [name],
        );
        self.run(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'best')",
            (recording, file),
        );
        let library_track = if library {
            self.run(
                "INSERT INTO library_track (recording_id, source_status) VALUES (?1, 'missing')",
                [recording],
            )
        } else {
            0
        };
        (file, LibraryTrackId(library_track))
    }

    /// Library tracks named `names`.
    fn tracks(&self, names: &[&'static str]) -> Vec<LibraryTrackId> {
        names.iter().map(|n| self.track(n, true).1).collect()
    }

    fn rows(&self, sql: &'static str) -> Vec<Vec<String>> {
        self.writer.call(move |c| Ok(dump(c, sql))).unwrap()
    }

    /// Everything a playlist operation writes, as text.
    fn state(&self) -> Vec<Vec<String>> {
        let mut rows = self.rows("SELECT * FROM playlist ORDER BY id");
        rows.extend(self.rows("SELECT * FROM playlist_entry ORDER BY id"));
        rows.extend(self.rows("SELECT * FROM crate ORDER BY id"));
        rows.extend(self.rows("SELECT * FROM crate_entry ORDER BY id"));
        rows
    }

    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, OpsError> + Send + 'static,
    ) -> T {
        self.writer.call(move |c| Ok(f(c))).unwrap().unwrap()
    }

    fn create(&self, name: &'static str) -> PlaylistId {
        self.call(move |c| create_on(c, name)).unwrap()
    }

    fn add(&self, id: PlaylistId, tracks: &[LibraryTrackId], at: Option<u32>) -> PlaylistChanged {
        let tracks = tracks.to_vec();
        self.call(move |c| add_on(c, id, &tracks, at)).unwrap()
    }

    /// The playlist's (entry, Library track) in order, checking the
    /// positions are dense from 0.
    fn order(&self, id: PlaylistId) -> Vec<(PlaylistEntryId, LibraryTrackId)> {
        let rows: Vec<(i64, i64, i64)> = self
            .writer
            .call(move |c| {
                c.prepare(
                    "SELECT id, library_track_id, position FROM playlist_entry
                     WHERE playlist_id = ?1 ORDER BY position, id",
                )?
                .query_map([id.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
            })
            .unwrap();
        for (at, row) in rows.iter().enumerate() {
            assert_eq!(row.2, at as i64, "positions are dense: {rows:?}");
        }
        rows.into_iter()
            .map(|(e, t, _)| (PlaylistEntryId(e), LibraryTrackId(t)))
            .collect()
    }

    fn tracks_of(&self, id: PlaylistId) -> Vec<LibraryTrackId> {
        self.order(id).into_iter().map(|(_, t)| t).collect()
    }

    fn undo(&self) -> String {
        match undo_last_via(&self.writer).unwrap() {
            UndoOutcome::Undone { operation } => operation.kind,
            other => panic!("{other:?}"),
        }
    }

    /// Reads an export as rekordbox's newest read.
    fn read(&self, xml: &str) {
        let rows = SnapshotRows::from_xml(&RekordboxXml::parse(xml.as_bytes()).unwrap());
        self.writer
            .call(move |c| replace_snapshot(c, &rows, |_, _, _| Ok(())))
            .unwrap();
    }

    /// What the relink does when it finds rekordbox track `track_id` at `file`.
    fn matched(&self, track_id: i64, file: i64) {
        self.run(
            "UPDATE rekordbox_track SET file_id = ?1, relink_method = 'path' WHERE track_id = ?2",
            (file, track_id),
        );
    }

    fn copy(&self, path: &[&str], name: &'static str) -> Done<CopiedFromRekordbox> {
        let path: Vec<String> = path.iter().map(|s| (*s).to_owned()).collect();
        self.call(move |c| copy_on(c, &path, name))
    }
}

/// A send of every Library track, named "Track <id>", with the app's
/// playlists, built as the writer builds it.
fn send(lib: &Lib, leave_out: &[LibraryTrackId]) -> crate::rekordbox_write::Outgoing {
    let leave_out = leave_out.to_vec();
    let (tracks, playlists) = lib
        .writer
        .call(move |c| {
            let ids: Vec<i64> = c
                .prepare("SELECT id FROM library_track ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let tracks: Vec<TrackInput> = ids
                .into_iter()
                .filter(|id| !leave_out.contains(&LibraryTrackId(*id)))
                .map(|id| TrackInput {
                    library_track: LibraryTrackId(id),
                    values: Values::Ready {
                        in_rekordbox: false,
                        attributes: vec![
                            ("Name".into(), format!("Track {id}")),
                            ("Location".into(), format!(r"C:\Music\{id}.mp3")),
                        ],
                        rekordbox_holds_other_file: None,
                        file_missing: false,
                    },
                })
                .collect();
            Ok((tracks, crate::send::playlist_tree(c)?))
        })
        .unwrap();
    build(&SendInput {
        tracks,
        playlists,
        ..SendInput::default()
    })
    .unwrap()
}

/// The names of the tracks a sent playlist lists, in the file's order, read
/// back from the file the way rekordbox reads it.
fn sent_names(xml: &[u8], path: &[&str]) -> Vec<String> {
    let parsed = RekordboxXml::parse(xml).unwrap();
    let (_, playlist) = parsed
        .playlists
        .playlists()
        .into_iter()
        .find(|(folders, p)| {
            let mut full = folders.clone();
            full.push(&p.name);
            full == path
        })
        .unwrap_or_else(|| panic!("no playlist {path:?}"));
    playlist
        .entries
        .iter()
        .map(|e| match e.target {
            EntryTarget::Track(i) => parsed.tracks[i].attrs.get("Name").unwrap().to_owned(),
            other => panic!("{other:?}"),
        })
        .collect()
}

fn named(tracks: &[LibraryTrackId]) -> Vec<String> {
    tracks.iter().map(|t| format!("Track {}", t.0)).collect()
}

// Order, repeats and the send.

#[test]
fn order_survives_a_send_the_xml_lists_the_tracks_in_position_order_repeats_kept() {
    let lib = Lib::new();
    let t = lib.tracks(&["a", "b", "c", "d"]);
    let arc = lib.create("Arc");
    // Added out of id order, with a repeat, then one put at the top.
    lib.add(arc, &[t[2], t[0], t[1], t[0]], None);
    lib.add(arc, &[t[3]], Some(0));
    let empty = lib.create("Later");
    let expected = vec![t[3], t[2], t[0], t[1], t[0]];
    assert_eq!(lib.tracks_of(arc), expected);

    let out = send(&lib, &[]);
    assert_eq!(
        sent_names(out.xml(), &["Playlists", "Arc"]),
        named(&expected)
    );
    // An empty playlist goes out empty, under Playlists too.
    assert_eq!(sent_names(out.xml(), &["Playlists", "Later"]), named(&[]));
    assert!(lib.order(empty).is_empty());
    // The app owns the Playlists folder: the after-send list sees them as its own.
    let app = lib.writer.call(|c| crate::after_send::app_tree(c)).unwrap();
    let read = crate::after_send::RekordboxTree::from_xml(&RekordboxXml::parse(out.xml()).unwrap());
    let recorded = crate::after_send::Recorded::of(
        &["Playlists", "Arc"],
        crate::after_send::StaleKind::Playlist,
    );
    assert!(crate::after_send::stale_playlists(&read, &app, &recorded).is_empty());
}

#[test]
fn a_playlist_entry_whose_track_cannot_be_sent_is_reported_with_its_place() {
    let lib = Lib::new();
    let t = lib.tracks(&["a", "b"]);
    let arc = lib.create("Arc");
    lib.add(arc, &[t[0], t[1], t[0]], None);

    let out = send(&lib, &[t[0]]);
    assert_eq!(sent_names(out.xml(), &["Playlists", "Arc"]), named(&[t[1]]));
    let left = out
        .left_out
        .iter()
        .find(|l| l.library_track == t[0])
        .unwrap();
    let places: Vec<(Vec<String>, usize)> = left
        .entries
        .iter()
        .map(|e| (e.path.clone(), e.position))
        .collect();
    let path = vec!["Playlists".to_owned(), "Arc".to_owned()];
    assert_eq!(places, [(path.clone(), 0), (path, 2)]);
}

#[test]
fn a_track_can_be_added_twice_and_at_a_place_the_rest_moving_down() {
    let lib = Lib::new();
    let t = lib.tracks(&["a", "b", "c"]);
    let arc = lib.create("Arc");
    let added = lib.add(arc, &[t[0], t[1]], None);
    assert_eq!((added.changed, added.skipped), (2, 0));
    assert!(added.operation_id.is_some());
    lib.add(arc, &[t[0], t[2]], Some(1));
    assert_eq!(lib.tracks_of(arc), [t[0], t[0], t[2], t[1]]);
    // Past the end: last.
    lib.add(arc, &[t[1]], Some(99));
    assert_eq!(lib.tracks_of(arc), [t[0], t[0], t[2], t[1], t[1]]);

    let before = lib.state();
    lib.add(arc, &[t[2]], Some(0));
    assert_eq!(lib.undo(), ADD_OPERATION);
    assert_eq!(lib.state(), before);

    // Only Library tracks; nothing is added when one isn't.
    let (_, outside) = lib.track("e", false);
    let refused = lib.call(move |c| add_on(c, arc, &[t[0], LibraryTrackId(outside.0 + 999)], None));
    assert!(matches!(refused, Err(PlaylistsError::TrackNotFound)));
    assert_eq!(lib.state(), before);
}

#[test]
fn reorder_moves_entries_as_one_operation_and_undo_restores_the_order_exactly() {
    let lib = Lib::new();
    let t = lib.tracks(&["a", "b", "c", "d", "e"]);
    let arc = lib.create("Arc");
    lib.add(arc, &t, None);
    let e: Vec<PlaylistEntryId> = lib.order(arc).into_iter().map(|(e, _)| e).collect();
    let (ea, eb, ed) = (e[0], e[1], e[3]);
    let before = lib.state();

    // d and b (picked out of order) go before a, in the playlist's order;
    // an entry that isn't in it is skipped.
    let moved = lib
        .call(move |c| move_on(c, arc, &[ed, eb, PlaylistEntryId(-5)], Some(ea)))
        .unwrap();
    assert_eq!((moved.changed, moved.skipped), (2, 1));
    assert_eq!(lib.tracks_of(arc), [t[1], t[3], t[0], t[2], t[4]]);
    assert_eq!(
        lib.writer
            .call(|c| c.query_row(
                "SELECT count(*) FROM operation WHERE kind = 'move_in_playlist'",
                [],
                |r| r.get::<_, i64>(0)
            ))
            .unwrap(),
        1
    );
    // To the end; and before one of the moved entries (the next that stays).
    lib.call(move |c| move_on(c, arc, &[ea], None)).unwrap();
    assert_eq!(lib.tracks_of(arc), [t[1], t[3], t[2], t[4], t[0]]);
    let ee = e[4];
    lib.call(move |c| move_on(c, arc, &[eb, ee], Some(ee)))
        .unwrap();
    assert_eq!(lib.tracks_of(arc), [t[3], t[2], t[1], t[4], t[0]]);

    // Undo, step by step, back to where it started, every row as it was.
    for _ in 0..3 {
        assert_eq!(lib.undo(), MOVE_OPERATION);
    }
    assert_eq!(lib.state(), before);

    // A `before` that isn't in the playlist is refused.
    let refused = lib.call(move |c| move_on(c, arc, &[ea], Some(PlaylistEntryId(-1))));
    assert!(matches!(refused, Err(PlaylistsError::EntryNotFound)));
    assert_eq!(lib.state(), before);
}

#[test]
fn removing_entries_closes_up_the_rest_and_undo_puts_them_back() {
    let lib = Lib::new();
    let t = lib.tracks(&["a", "b", "c"]);
    let arc = lib.create("Arc");
    lib.add(arc, &[t[0], t[1], t[0], t[2]], None);
    let e: Vec<PlaylistEntryId> = lib.order(arc).into_iter().map(|(e, _)| e).collect();
    let before = lib.state();

    // One of the two places of a; the other stays.
    let removed = lib.call(move |c| remove_on(c, arc, &[e[0], e[1]])).unwrap();
    assert_eq!((removed.changed, removed.skipped), (2, 0));
    assert_eq!(lib.tracks_of(arc), [t[0], t[2]]);
    assert_eq!(lib.undo(), REMOVE_OPERATION);
    assert_eq!(lib.state(), before);
}

#[test]
fn deleting_a_playlist_deletes_its_entries_and_undo_brings_both_back() {
    let lib = Lib::new();
    let t = lib.tracks(&["a", "b"]);
    let arc = lib.create("Arc");
    lib.add(arc, &[t[1], t[0], t[1]], None);
    let before = lib.state();

    let deleted = lib.call(move |c| delete_on(c, arc)).unwrap();
    assert!(deleted.is_some());
    assert!(lib.rows("SELECT * FROM playlist").is_empty());
    assert!(lib.rows("SELECT * FROM playlist_entry").is_empty());
    assert_eq!(
        lib.rows("SELECT count(*) FROM library_track"),
        [["Integer(2)"]]
    );
    assert_eq!(lib.undo(), DELETE_OPERATION);
    assert_eq!(lib.state(), before);
    assert!(matches!(
        lib.call(move |c| delete_on(c, PlaylistId(arc.0 + 1))),
        Err(PlaylistsError::NotFound)
    ));
}

#[test]
fn a_name_is_trimmed_and_never_one_a_send_would_refuse() {
    let lib = Lib::new();
    let arc = lib.create("  Arc ");
    assert_eq!(lib.call(|c| Ok(list(c)?))[0].name, "Arc");
    let taken = |name: &'static str| lib.call(move |c| create_on(c, name));
    // The writer's sibling rule: letter case and trailing space don't count.
    assert!(
        matches!(taken("arc"), Err(PlaylistsError::NameTaken { existing }) if existing == "Arc")
    );
    // The tag playlists' folder sits beside them.
    assert!(matches!(
        taken("TAGS"),
        Err(PlaylistsError::NameTaken { .. })
    ));
    assert!(matches!(taken("   "), Err(PlaylistsError::NameEmpty)));
    assert!(matches!(
        taken("Bad\u{1}"),
        Err(PlaylistsError::NameUnsendable)
    ));

    // Renaming: to its own name in another case is fine, to another's not.
    let other = lib.create("Other");
    assert!(lib
        .call(move |c| rename_on(c, arc, "ARC"))
        .unwrap()
        .is_some());
    assert!(matches!(
        lib.call(move |c| rename_on(c, other, "arc")),
        Err(PlaylistsError::NameTaken { .. })
    ));
    assert_eq!(lib.undo(), RENAME_OPERATION);
    assert_eq!(lib.call(|c| Ok(list(c)?))[0].name, "Arc");
    // The database holds the rule too, so nothing can bring a taken name back.
    let twice = lib.writer.call(|c| {
        Ok(c.execute(
            "INSERT INTO playlist (name, name_key) VALUES ('ARC', 'arc')",
            [],
        ))
    });
    assert!(twice.unwrap().is_err());
}

// Conversions.

#[test]
fn save_as_crate_makes_a_crate_of_each_track_once_and_undo_takes_it_away() {
    let lib = Lib::new();
    let t = lib.tracks(&["a", "b", "c"]);
    let arc = lib.create("Arc");
    lib.add(arc, &[t[2], t[0], t[2], t[1]], None);
    let before = lib.state();

    let crate_id = lib
        .call(move |c| save_as_crate_on(c, arc, "Arc crate"))
        .unwrap();
    let mut held = lib
        .writer
        .call(move |c| crate::crates::track_ids(c, crate_id))
        .unwrap()
        .unwrap();
    held.sort();
    assert_eq!(held, t);
    // The playlist is left as it was.
    assert_eq!(lib.tracks_of(arc), [t[2], t[0], t[2], t[1]]);
    assert_eq!(lib.undo(), SAVE_AS_CRATE_OPERATION);
    assert_eq!(lib.state(), before);

    // The crate's own name rules apply.
    lib.call(|c| crate::crates::create_on(c, "Taken")).unwrap();
    assert!(matches!(
        lib.call(move |c| save_as_crate_on(c, arc, "taken")),
        Err(PlaylistsError::Crate(CratesError::NameTaken { .. }))
    ));
}

#[test]
fn a_playlist_from_a_crate_takes_the_order_given_then_the_rest_and_undo_takes_it_away() {
    let lib = Lib::new();
    let t = lib.tracks(&["a", "b", "c", "d"]);
    let crate_id = lib.call(|c| crate::crates::create_on(c, "Crate")).unwrap();
    let members = vec![t[0], t[1], t[2]];
    lib.call(move |c| crate::crates::add_on(c, crate_id, &members))
        .unwrap();
    let before = lib.state();

    // The view's sort names c, then a, and d (not in the crate); b follows.
    let order = vec![t[2], t[3], t[0]];
    let made = lib
        .call(move |c| from_crate_on(c, crate_id, "From crate", &order))
        .unwrap();
    assert_eq!(lib.tracks_of(made), [t[2], t[0], t[1]]);
    assert_eq!(lib.undo(), FROM_CRATE_OPERATION);
    assert_eq!(lib.state(), before);

    // A smart crate gives what its rule holds now.
    let rule = crate::crates::smart::SmartRule {
        combine: crate::crates::smart::Combine::All,
        conditions: vec![],
    };
    let smart = lib
        .call(move |c| crate::crates::smart::create_smart_on(c, None, "Smart", &rule))
        .unwrap();
    let held = lib
        .writer
        .call(move |c| crate::crates::smart::membership::evaluate(c, smart))
        .unwrap();
    let made = lib
        .call(move |c| from_crate_on(c, smart, "From smart", &[]))
        .unwrap();
    assert_eq!(lib.tracks_of(made), held);

    assert!(matches!(
        lib.call(move |c| from_crate_on(c, CrateId(crate_id.0 + 99), "None", &[])),
        Err(PlaylistsError::Crate(CratesError::NotFound))
    ));
}

// The Library track's own paths.

#[test]
fn removing_a_track_from_the_library_takes_its_entries_and_undo_puts_them_back() {
    let lib = Lib::new();
    let t = lib.tracks(&["a", "b", "c"]);
    let arc = lib.create("Arc");
    lib.add(arc, &[t[0], t[1], t[0], t[2]], None);
    let other = lib.create("Other");
    lib.add(other, &[t[0]], None);
    let before = lib.state();

    crate::library::remove(&lib.writer, t[0]).unwrap();
    assert_eq!(lib.tracks_of(arc), [t[1], t[2]]);
    assert!(lib.tracks_of(other).is_empty());
    assert_eq!(lib.undo(), crate::library::REMOVE_OPERATION);
    assert_eq!(lib.state(), before);
}

#[test]
fn the_view_lists_every_entry_in_order_with_its_row() {
    let lib = Lib::new();
    let t = lib.tracks(&["a", "b"]);
    let arc = lib.create("Arc");
    lib.add(arc, &[t[1], t[0], t[1]], None);
    let view = lib
        .writer
        .call(move |c| view(c, &crate::scan::system_volumes(), arc))
        .unwrap()
        .unwrap();
    assert_eq!(view.playlist.track_count, 3);
    assert_eq!(view.playlist.origin, PlaylistOrigin::App);
    let shown: Vec<(u32, Option<LibraryTrackId>)> = view
        .items
        .iter()
        .map(|i| (i.position, i.row.library_track_id))
        .collect();
    assert_eq!(shown, [(0, Some(t[1])), (1, Some(t[0])), (2, Some(t[1]))]);
}

// From rekordbox.

/// An export whose playlist "Folder/Set" lists TrackIDs 3, 1, 2, 1, 4 and a
/// key no track has; with `reversed`, the other way round.
fn export(reversed: bool) -> String {
    let mut keys = ["3", "1", "2", "1", "4", "99"];
    if reversed {
        keys.reverse();
    }
    let entries: String = keys
        .iter()
        .map(|k| format!(r#"<TRACK Key="{k}"/>"#))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="4">
    <TRACK TrackID="1" Name="One" Location="file://localhost/E:/Music/a.mp3"/>
    <TRACK TrackID="2" Name="Two" Location="file://localhost/E:/Music/b.mp3"/>
    <TRACK TrackID="3" Name="Three" Location="file://localhost/E:/Music/c.mp3"/>
    <TRACK TrackID="4" Name="Four" Location="file://localhost/E:/Music/d.mp3"/>
  </COLLECTION>
  <PLAYLISTS><NODE Type="0" Name="ROOT" Count="1">
    <NODE Type="0" Name="Folder" Count="1">
      <NODE Name="Set" Type="1" KeyType="0" Entries="6">{entries}</NODE>
    </NODE>
  </NODE></PLAYLISTS>
</DJ_PLAYLISTS>
"#
    )
}

#[test]
fn a_rekordbox_playlist_becomes_an_app_playlist_in_rekordboxs_order_library_tracks_only() {
    let lib = Lib::new();
    let (fa, a) = lib.track("a", true);
    let (fb, b) = lib.track("b", true);
    let (fc, c) = lib.track("c", true);
    // d is matched but not in the Library: left out and counted.
    let (fd, _) = lib.track("d", false);
    lib.read(&export(false));
    for (track_id, file) in [(1, fa), (2, fb), (3, fc), (4, fd)] {
        lib.matched(track_id, file);
    }
    assert_eq!(
        lib.writer.call(|c| rekordbox_playlists(c)).unwrap(),
        [super::import::RekordboxPlaylist {
            path: vec!["Folder".into(), "Set".into()],
            entries: 6,
        }]
    );
    let before = lib.state();

    let copied = lib.copy(&["Folder", "Set"], "Set").unwrap();
    assert_eq!((copied.added, copied.not_in_library), (4, 2));
    assert_eq!(lib.tracks_of(copied.playlist_id), [c, a, b, a]);
    let listed = lib.call(|c| Ok(list(c)?));
    assert_eq!(listed[0].origin, PlaylistOrigin::Rekordbox);
    assert_eq!(
        listed[0].source_path,
        Some(vec!["Folder".to_owned(), "Set".to_owned()])
    );
    assert_eq!(lib.undo(), FROM_REKORDBOX_OPERATION);
    assert_eq!(lib.state(), before);

    // A probable match isn't trusted.
    lib.run(
        "UPDATE rekordbox_track SET relink_probable = 1 WHERE track_id = 3",
        (),
    );
    let copied = lib.copy(&["Folder", "Set"], "Set").unwrap();
    assert_eq!(lib.tracks_of(copied.playlist_id), [a, b, a]);

    assert!(matches!(
        lib.copy(&["Folder", "Nope"], "Nope"),
        Err(PlaylistsError::RekordboxPlaylistNotFound)
    ));
}

#[test]
fn a_new_read_never_changes_a_copied_playlist_and_copying_again_makes_another() {
    let lib = Lib::new();
    let (fa, a) = lib.track("a", true);
    let (fb, b) = lib.track("b", true);
    let (fc, c) = lib.track("c", true);
    lib.read(&export(false));
    for (track_id, file) in [(1, fa), (2, fb), (3, fc)] {
        lib.matched(track_id, file);
    }
    let first = lib.copy(&["Folder", "Set"], "Set").unwrap().playlist_id;
    let kept = lib.state();

    // rekordbox's playlist is reversed, then a read of it.
    lib.read(&export(true));
    for (track_id, file) in [(1, fa), (2, fb), (3, fc)] {
        lib.matched(track_id, file);
    }
    assert_eq!(lib.state(), kept);
    let again = lib
        .copy(&["Folder", "Set"], "Set again")
        .unwrap()
        .playlist_id;
    assert_ne!(again, first);
    assert_eq!(lib.tracks_of(again), [a, b, a, c]);
    assert_eq!(lib.tracks_of(first), [c, a, b, a]);
}

#[test]
fn every_read_replaces_rekordboxs_playlists_with_their_order() {
    let lib = Lib::new();
    let stored = |lib: &Lib| {
        lib.rows("SELECT path, position, track_id FROM rekordbox_playlist_entry ORDER BY id")
            .into_iter()
            .map(|r| r[2].clone())
            .collect::<Vec<_>>()
    };
    lib.read(&export(false));
    let ids = |s: &[&str]| -> Vec<String> {
        s.iter()
            .map(|v| {
                if *v == "-" {
                    "Null".to_owned()
                } else {
                    format!("Integer({v})")
                }
            })
            .collect()
    };
    assert_eq!(stored(&lib), ids(&["3", "1", "2", "1", "4", "-"]));
    lib.read(&export(true));
    assert_eq!(stored(&lib), ids(&["-", "4", "1", "2", "1", "3"]));
    // The snapshot is only replaced, never edited.
    let edited = lib.writer.call(|c| {
        Ok(c.execute(
            "UPDATE rekordbox_playlist_entry SET position = 9 WHERE id = 1",
            [],
        ))
    });
    assert!(edited.unwrap().is_err());
}

#[test]
fn undoing_a_rename_whose_old_name_was_taken_since_says_the_name_is_taken() {
    let lib = Lib::new();
    let first = lib.create("Old");
    lib.create("Other");
    lib.call(move |c| rename_on(c, first, "New")).unwrap();
    // Another program's write, outside the log, takes the old name.
    lib.run(
        "UPDATE playlist SET name = 'OLD', name_key = 'old' WHERE id = 2",
        (),
    );
    let before = lib.state();
    match undo_last_via(&lib.writer).unwrap() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(
            reason,
            crate::ops::UndoRefusal::CrateNameTaken {
                name: "OLD".to_owned()
            }
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(lib.state(), before);
}
