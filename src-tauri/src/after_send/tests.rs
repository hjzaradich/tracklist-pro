//! The after-send lists over a migrated database in a temp dir. Every
//! name, path and track is synthetic.

use super::*;
use crate::db::Writer;
use crate::rekordbox::source::LAST_READ;
use crate::rekordbox::RekordboxXml;

fn playlist(name: &str) -> TreeNode {
    TreeNode::playlist(name)
}

fn folder(name: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode::folder(name, children)
}

fn paths(stale: &[StalePlaylist]) -> Vec<String> {
    stale.iter().map(|s| s.path.join(" / ")).collect()
}

// ---- stale playlists -------------------------------------------------------

#[test]
fn a_renamed_crate_lists_its_old_name_in_rekordbox() {
    let read = RekordboxTree {
        crates: vec![playlist("Warm up"), playlist("Peak")],
        playlists: vec![],
    };
    let app = AppTree {
        crates: vec![playlist("Warm up (new)"), playlist("Peak")],
        playlists: vec![],
    };
    let stale = stale_playlists(&read, &app, &Recorded::everything_in(&read));
    assert_eq!(paths(&stale), ["Crates / Warm up"]);
    assert_eq!(stale[0].kind, StaleKind::Playlist);
}

#[test]
fn a_crate_deleted_in_the_app_is_listed_inside_its_folder() {
    let read = RekordboxTree {
        crates: vec![folder("House", vec![playlist("Deep"), playlist("Old")])],
        playlists: vec![],
    };
    let app = AppTree {
        crates: vec![folder("House", vec![playlist("Deep")])],
        playlists: vec![],
    };
    assert_eq!(
        paths(&stale_playlists(
            &read,
            &app,
            &Recorded::everything_in(&read)
        )),
        ["Crates / House / Old"]
    );
}

#[test]
fn a_folder_gone_from_the_app_is_listed_once_with_how_many_playlists_it_holds() {
    let read = RekordboxTree {
        crates: vec![folder(
            "Old",
            vec![playlist("A"), folder("Deeper", vec![playlist("B")])],
        )],
        playlists: vec![],
    };
    let stale = stale_playlists(&read, &AppTree::default(), &Recorded::everything_in(&read));
    assert_eq!(paths(&stale), ["Crates / Old"]);
    assert_eq!(stale[0].kind, StaleKind::Folder);
    assert_eq!(stale[0].playlists_inside, 2);
}

#[test]
fn a_name_differing_only_by_case_unicode_form_or_trailing_spaces_is_not_stale() {
    let read = RekordboxTree {
        crates: vec![
            playlist("Caf\u{e9} Night"),
            playlist("PEAK"),
            playlist("Closing  "),
        ],
        playlists: vec![],
    };
    let app = AppTree {
        crates: vec![
            playlist("Cafe\u{301} night"),
            playlist("peak"),
            playlist("Closing"),
        ],
        playlists: vec![],
    };
    assert_eq!(
        stale_playlists(&read, &app, &Recorded::everything_in(&read)),
        []
    );
}

#[test]
fn a_folder_is_never_taken_for_a_playlist_of_the_same_name() {
    let read = RekordboxTree {
        crates: vec![folder("Same", vec![])],
        playlists: vec![],
    };
    let app = AppTree {
        crates: vec![playlist("Same")],
        playlists: vec![],
    };
    assert_eq!(
        paths(&stale_playlists(
            &read,
            &app,
            &Recorded::everything_in(&read)
        )),
        ["Crates / Same"]
    );
}

#[test]
fn an_empty_playlist_and_an_empty_folder_in_rekordbox_are_still_listed() {
    let read = RekordboxTree {
        crates: vec![playlist("Empty"), folder("Hollow", vec![])],
        playlists: vec![],
    };
    let stale = stale_playlists(&read, &AppTree::default(), &Recorded::everything_in(&read));
    assert_eq!(paths(&stale), ["Crates / Empty", "Crates / Hollow"]);
    assert_eq!(stale[1].playlists_inside, 0);
}

#[test]
fn the_playlists_folder_is_checked_apart_from_the_crates_folder() {
    let read = RekordboxTree {
        crates: vec![playlist("Mine")],
        playlists: vec![playlist("Mine"), playlist("Other")],
    };
    let app = AppTree {
        crates: vec![playlist("Mine")],
        playlists: vec![],
    };
    assert_eq!(
        paths(&stale_playlists(
            &read,
            &app,
            &Recorded::everything_in(&read)
        )),
        ["Playlists / Mine", "Playlists / Other"]
    );
}

const EXPORT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <COLLECTION Entries="0"/>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="4">
      <NODE Type="1" Name="Own playlist" KeyType="0" Entries="0"/>
      <NODE Type="0" Name="Own folder" Count="1">
        <NODE Type="1" Name="Inside own folder" KeyType="0" Entries="0"/>
      </NODE>
      <NODE Type="0" Name="Crates" Count="2">
        <NODE Type="1" Name="Old name" KeyType="0" Entries="0"/>
        <NODE Type="0" Name="House" Count="1">
          <NODE Type="1" Name="Deep" KeyType="0" Entries="0"/>
        </NODE>
      </NODE>
      <NODE Type="0" Name="Playlists" Count="0"/>
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>"#;

#[test]
fn a_playlist_outside_the_crates_and_playlists_folders_is_never_listed() {
    let xml = RekordboxXml::parse(EXPORT.as_bytes()).unwrap();
    let read = RekordboxTree::from_xml(&xml);
    assert_eq!(
        read,
        RekordboxTree {
            crates: vec![
                playlist("Old name"),
                folder("House", vec![playlist("Deep")])
            ],
            playlists: vec![],
        }
    );
    // rekordbox's own playlists and folders at the top never show up.
    let stale = stale_playlists(&read, &AppTree::default(), &Recorded::everything_in(&read));
    assert_eq!(paths(&stale), ["Crates / Old name", "Crates / House"]);
    assert!(stale
        .iter()
        .all(|s| matches!(s.path[0].as_str(), "Crates" | "Playlists")));
}

#[test]
fn a_tree_built_from_what_a_send_writes_keeps_its_folders_and_playlists() {
    use crate::library::LibraryTrackId;
    use crate::rekordbox_write::Node;
    let nodes = vec![Node::Folder {
        name: "House".into(),
        children: vec![Node::Playlist {
            name: "Deep".into(),
            entries: vec![LibraryTrackId(1)],
        }],
    }];
    assert_eq!(
        RekordboxTree::from_nodes(&nodes, &[]).crates,
        vec![folder("House", vec![playlist("Deep")])]
    );
}

// ---- the database ----------------------------------------------------------

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

    fn run(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, params)?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    fn lists(&self) -> AfterSendLists {
        self.writer.call(|c| lists(c)).unwrap()
    }

    /// A removed track that was sent to `location` and removed at `at`.
    fn removed(&self, title: &str, location: Option<&str>, at: &str) -> i64 {
        let recording = self.run(
            "INSERT INTO recording (title, artist) VALUES (?1, 'Synthetic Artist')",
            (title.to_owned(),),
        );
        self.run(
            "INSERT INTO library_removal (recording_id, removed_at, last_sent_location, last_exported_at)
             VALUES (?1, ?2, ?3, '2026-09-30T10:00:00.000Z')",
            (recording, at.to_owned(), location.map(str::to_owned)),
        );
        recording
    }

    /// A rekordbox read made at `at`, holding these Locations.
    fn read(&self, at: &str, held: &[&str], complete: bool) {
        let at = at.to_owned();
        let held: Vec<String> = held.iter().map(|l| (*l).to_owned()).collect();
        self.writer
            .call(move |c| {
                c.execute("DELETE FROM rekordbox_track", [])?;
                for (i, raw) in held.iter().enumerate() {
                    let key = crate::rekordbox::location::decode(raw).unwrap().match_key();
                    c.execute(
                        "INSERT INTO rekordbox_track (attributes, location_key, read_at)
                         VALUES (?1, ?2, ?3)",
                        (
                            format!("{{\"TrackID\":\"{}\",\"Location\":\"{raw}\"}}", i + 1),
                            key,
                            &at,
                        ),
                    )?;
                }
                let last = serde_json::json!({
                    "path": "export.xml", "modifiedMs": 0, "readAt": at,
                    "summary": {"tracks": held.len(), "streaming": 0, "kept": 0,
                                "notStored": 0, "complete": complete}
                });
                c.execute(
                    "INSERT INTO setting (key, value) VALUES (?1, ?2)
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    (LAST_READ, last.to_string()),
                )
            })
            .unwrap();
    }
}

const A: &str = "file://localhost/E:/Music/Synthetic%20A.mp3";
const B: &str = "file://localhost/E:/Music/b#(1).mp3";

#[test]
fn a_removed_track_drops_off_the_list_after_a_read_without_it() {
    let lib = Lib::new();
    lib.removed("Synthetic A", Some(A), "2026-10-01T10:00:00.000Z");
    // A read from before the removal says nothing about it.
    lib.read("2026-10-01T09:00:00.000Z", &[], true);
    assert_eq!(lib.lists().manual_removals.len(), 1);
    // A later read that still has it keeps it listed.
    lib.read("2026-10-01T11:00:00.000Z", &[A], true);
    let listed = lib.lists().manual_removals;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title.as_deref(), Some("Synthetic A"));
    assert_eq!(listed[0].path.as_deref(), Some(r"E:\Music\Synthetic A.mp3"));
    // A later read without it takes it off.
    lib.read("2026-10-01T12:00:00.000Z", &[], true);
    assert_eq!(lib.lists().manual_removals, []);
}

#[test]
fn a_removed_track_is_matched_to_rekordbox_by_location_however_it_is_spelled() {
    let lib = Lib::new();
    lib.removed(
        "Synthetic B",
        Some("file://localhost/E:/Music/b%23%28%31%29.mp3"),
        "2026-10-01T10:00:00.000Z",
    );
    lib.read("2026-10-01T11:00:00.000Z", &[B], true);
    assert_eq!(lib.lists().manual_removals.len(), 1);
}

impl Lib {
    /// A file of `recording`, and rekordbox's entry at `location` matched
    /// to it: trusted, or only probable.
    fn entry_for(&self, recording: i64, location: &'static str, probable: bool) {
        self.writer
            .call(move |c| {
                c.execute_batch(
                    "INSERT OR IGNORE INTO volume (id, identity, kind, last_mount_path)
                         VALUES (1, 'serial=NTFS-1A2B3C4D', 'external', 'E:\\');
                     INSERT OR IGNORE INTO music_folder (id, volume_id, rel_path, rel_path_key)
                         VALUES (1, 1, 'Music', 'Music');",
                )?;
                let name = format!("{recording}-{probable}.mp3");
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
                     VALUES (1, ?1, ?1, 1)",
                    [&name],
                )?;
                let file = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'best')",
                    (recording, file),
                )?;
                let key = crate::rekordbox::location::decode(location).unwrap().match_key();
                c.execute(
                    "INSERT INTO rekordbox_track
                         (attributes, location_key, read_at, file_id, relink_method, relink_probable)
                     VALUES (?1, ?2, '2026-10-01T09:00:00.000Z', ?3, ?4, ?5)",
                    (
                        format!("{{\"TrackID\":\"{file}\",\"Location\":\"{location}\"}}"),
                        key,
                        file,
                        if probable { "filename_only" } else { "path" },
                        probable,
                    ),
                )
            })
            .unwrap();
    }
}

#[test]
fn a_removed_track_that_was_never_sent_is_listed_while_rekordbox_holds_its_own_entry() {
    let lib = Lib::new();
    // A read from before the removal is enough: the entry is rekordbox's
    // own, not something a send may or may not have put there.
    lib.read("2026-10-01T09:00:00.000Z", &[], true);
    let recording = lib.removed("Never sent", None, "2026-10-01T10:00:00.000Z");
    lib.entry_for(recording, A, false);

    let listed = lib.lists().manual_removals;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title.as_deref(), Some("Never sent"));
    assert_eq!(listed[0].path.as_deref(), Some(r"E:\Music\Synthetic A.mp3"));

    // The user deletes it in rekordbox: the next read has no entry for it.
    lib.read("2026-10-01T12:00:00.000Z", &[], true);
    assert_eq!(lib.lists().manual_removals, []);
}

#[test]
fn a_never_sent_removed_track_rekordbox_has_no_entry_for_is_not_listed() {
    let lib = Lib::new();
    lib.removed("Never sent", None, "2026-10-01T10:00:00.000Z");
    // rekordbox holds something, but nothing matched to this track.
    lib.read("2026-10-01T11:00:00.000Z", &[A], true);
    assert_eq!(lib.lists().manual_removals, []);
}

#[test]
fn a_never_sent_removed_track_with_only_a_probable_match_is_not_listed() {
    let lib = Lib::new();
    lib.read("2026-10-01T09:00:00.000Z", &[], true);
    let recording = lib.removed("Maybe theirs", None, "2026-10-01T10:00:00.000Z");
    lib.entry_for(recording, A, true);
    assert_eq!(lib.lists().manual_removals, []);
}

#[test]
fn sent_and_never_sent_removals_are_listed_together_in_the_order_removed() {
    let lib = Lib::new();
    lib.read("2026-10-01T09:00:00.000Z", &[], true);
    let never = lib.removed("Never sent", None, "2026-10-01T10:00:00.000Z");
    lib.entry_for(never, B, false);
    lib.removed("Synthetic A", Some(A), "2026-10-01T10:30:00.000Z");
    let titles: Vec<_> = lib
        .lists()
        .manual_removals
        .into_iter()
        .map(|r| r.title.unwrap())
        .collect();
    assert_eq!(titles, ["Never sent", "Synthetic A"]);
}

#[test]
fn with_no_rekordbox_read_at_all_every_sent_removal_is_listed() {
    let lib = Lib::new();
    lib.removed("Synthetic A", Some(A), "2026-10-01T10:00:00.000Z");
    assert_eq!(lib.lists().manual_removals.len(), 1);
}

#[test]
fn removals_come_in_the_order_they_were_removed() {
    let lib = Lib::new();
    lib.removed("First", Some(A), "2026-10-01T10:00:00.000Z");
    lib.removed("Second", Some(B), "2026-10-01T10:05:00.000Z");
    let titles: Vec<_> = lib
        .lists()
        .manual_removals
        .into_iter()
        .map(|r| r.title.unwrap())
        .collect();
    assert_eq!(titles, ["First", "Second"]);
}

#[test]
fn the_apps_own_tree_comes_from_the_crate_table_with_folders_nested() {
    let lib = Lib::new();
    let house = lib.run(
        "INSERT INTO crate (kind, name) VALUES ('folder', 'House')",
        (),
    );
    lib.run(
        "INSERT INTO crate (parent_id, kind, name) VALUES (?1, 'static', 'Deep')",
        (house,),
    );
    let tree = lib.writer.call(|c| app_tree(c)).unwrap();
    assert_eq!(tree.crates, vec![folder("House", vec![playlist("Deep")])]);
    assert_eq!(tree.playlists, []);
}

#[test]
fn a_second_read_replaces_the_kept_playlist_names() {
    let lib = Lib::new();
    let one = RekordboxTree {
        crates: vec![playlist("One")],
        playlists: vec![],
    };
    let two = RekordboxTree {
        crates: vec![playlist("Two")],
        playlists: vec![],
    };
    let (a, b) = (one, two.clone());
    lib.writer.call(move |c| save_tree(c, &a)).unwrap();
    lib.writer.call(move |c| save_tree(c, &b)).unwrap();
    assert_eq!(lib.writer.call(|c| load_tree(c)).unwrap(), Some(two));
}

// ---- only what the app itself sent -------------------------------------------

use crate::rekordbox_write::{record_send, SentPath};

impl Lib {
    /// A send that wrote these folders (`true`) and playlists.
    fn sent(&self, paths: &[(&[&str], bool)]) {
        let paths: Vec<SentPath> = paths
            .iter()
            .map(|(path, folder)| SentPath {
                path: path.iter().map(|n| (*n).to_owned()).collect(),
                folder: *folder,
            })
            .collect();
        self.writer
            .call(move |c| record_send(c, &[], &paths))
            .unwrap();
    }

    /// A rekordbox read that holds this tree under its two folders.
    fn read_tree(&self, tree: RekordboxTree) {
        self.writer.call(move |c| save_tree(c, &tree)).unwrap();
    }

    fn crate_named(&self, name: &'static str) {
        self.run(
            "INSERT INTO crate (kind, name) VALUES ('static', ?1)",
            (name,),
        );
    }

    fn recorded_count(&self) -> i64 {
        self.writer
            .call(|c| c.query_row("SELECT count(*) FROM sent_playlist", [], |r| r.get(0)))
            .unwrap()
    }
}

fn crates(nodes: Vec<TreeNode>) -> RekordboxTree {
    RekordboxTree {
        crates: nodes,
        playlists: vec![],
    }
}

#[test]
fn playlists_are_unchecked_until_a_read_has_kept_them_and_checked_after() {
    let lib = Lib::new();
    let before = lib.lists();
    assert!(!before.playlists_checked);
    assert_eq!(before.stale_playlists, []);
    lib.read_tree(RekordboxTree::default());
    assert!(lib.lists().playlists_checked);
}

#[test]
fn with_nothing_recorded_from_a_first_send_nothing_is_stale() {
    let lib = Lib::new();
    lib.crate_named("Kept");
    let xml = RekordboxXml::parse(EXPORT.as_bytes()).unwrap();
    lib.read_tree(RekordboxTree::from_xml(&xml));
    // rekordbox has `Old name` and `House` under Crates, the app has only
    // `Kept`, and no send has recorded anything: they're the user's own.
    let lists = lib.lists();
    assert!(lists.playlists_checked);
    assert_eq!(lists.stale_playlists, []);
}

#[test]
fn a_users_own_playlist_inside_playlists_is_never_listed() {
    let lib = Lib::new();
    lib.sent(&[(&["Crates", "Mine"], false)]);
    lib.read_tree(RekordboxTree {
        crates: vec![playlist("Mine")],
        playlists: vec![
            playlist("Made by hand"),
            folder("My folder", vec![playlist("Also mine")]),
        ],
    });
    lib.crate_named("Mine");
    assert_eq!(lib.lists().stale_playlists, []);
}

#[test]
fn a_sent_then_deleted_crate_is_listed_until_a_read_no_longer_has_it() {
    let lib = Lib::new();
    lib.sent(&[(&["Crates", "Warm up"], false)]);
    // Imported: rekordbox shows it. The app no longer has the crate.
    lib.read_tree(crates(vec![playlist("Warm up"), playlist("User's own")]));
    assert_eq!(paths(&lib.lists().stale_playlists), ["Crates / Warm up"]);
    // Still there on the next read: still listed.
    lib.read_tree(crates(vec![playlist("Warm up")]));
    assert_eq!(paths(&lib.lists().stale_playlists), ["Crates / Warm up"]);
    // The user deleted it in rekordbox: gone from the list and the record.
    lib.read_tree(crates(vec![]));
    assert_eq!(lib.lists().stale_playlists, []);
    assert_eq!(lib.recorded_count(), 0);
}

#[test]
fn a_sent_then_renamed_crate_lists_its_old_name() {
    let lib = Lib::new();
    lib.sent(&[(&["Crates", "Warm up"], false)]);
    lib.crate_named("Warm up (new)");
    // The next send wrote the new name too.
    lib.sent(&[(&["Crates", "Warm up (new)"], false)]);
    lib.read_tree(crates(vec![playlist("Warm up"), playlist("Warm up (new)")]));
    assert_eq!(paths(&lib.lists().stale_playlists), ["Crates / Warm up"]);
}

#[test]
fn a_path_never_seen_in_rekordbox_is_not_listed_and_is_kept_for_when_the_send_is_imported() {
    let lib = Lib::new();
    // Sent, then deleted in the app before the user imported anything.
    lib.sent(&[(&["Crates", "Never imported"], false)]);
    lib.read_tree(crates(vec![]));
    assert_eq!(lib.lists().stale_playlists, []);
    assert_eq!(
        lib.recorded_count(),
        1,
        "not forgotten: it may be imported later"
    );
    // The import happens and the next read shows it: now it's stale.
    lib.read_tree(crates(vec![playlist("Never imported")]));
    assert_eq!(
        paths(&lib.lists().stale_playlists),
        ["Crates / Never imported"]
    );
}

#[test]
fn a_crate_the_app_has_again_is_not_stale_and_a_path_sent_again_is_one_record() {
    let lib = Lib::new();
    lib.sent(&[(&["Crates", "Warm up"], false)]);
    lib.read_tree(crates(vec![playlist("Warm up")]));
    assert_eq!(lib.lists().stale_playlists.len(), 1);
    lib.crate_named("WARM UP");
    lib.sent(&[(&["Crates", "WARM UP"], false)]);
    assert_eq!(lib.lists().stale_playlists, []);
    assert_eq!(lib.recorded_count(), 1);
}

#[test]
fn a_sent_crate_inside_a_folder_the_user_made_is_still_found() {
    let lib = Lib::new();
    lib.sent(&[(&["Crates", "Theirs", "Sent one"], false)]);
    lib.read_tree(crates(vec![folder("Theirs", vec![playlist("Sent one")])]));
    assert_eq!(
        paths(&lib.lists().stale_playlists),
        ["Crates / Theirs / Sent one"]
    );
}

#[test]
fn a_folder_and_a_playlist_of_one_name_are_recorded_apart() {
    let lib = Lib::new();
    lib.sent(&[(&["Crates", "Same"], true)]);
    // The user's own playlist of that name isn't the sent folder.
    lib.read_tree(crates(vec![playlist("Same")]));
    assert_eq!(lib.lists().stale_playlists, []);
}

#[test]
fn a_sent_folder_gone_from_the_app_is_listed_whole_only_when_everything_in_it_was_sent() {
    let lib = Lib::new();
    lib.sent(&[
        (&["Crates", "Old"], true),
        (&["Crates", "Old", "Sent one"], false),
        (&["Crates", "Old", "Deeper"], true),
        (&["Crates", "Old", "Deeper", "Sent two"], false),
    ]);
    lib.read_tree(crates(vec![folder(
        "Old",
        vec![
            playlist("Sent one"),
            folder("Deeper", vec![playlist("Sent two")]),
        ],
    )]));
    let stale = lib.lists().stale_playlists;
    assert_eq!(paths(&stale), ["Crates / Old"]);
    assert_eq!(stale[0].kind, StaleKind::Folder);
    // Exactly what deleting the folder deletes.
    assert_eq!(stale[0].playlists_inside, 2);
}

#[test]
fn a_sent_folder_holding_something_the_user_made_is_never_listed_only_what_was_sent_inside_it() {
    let lib = Lib::new();
    lib.sent(&[
        (&["Crates", "Old"], true),
        (&["Crates", "Old", "Sent one"], false),
        (&["Crates", "Old", "Deeper"], true),
        (&["Crates", "Old", "Deeper", "Sent two"], false),
    ]);
    // The user's own playlist sits deep inside: deleting `Old` or `Deeper`
    // would delete it.
    lib.read_tree(crates(vec![folder(
        "Old",
        vec![
            playlist("Sent one"),
            folder("Deeper", vec![playlist("Sent two"), playlist("User's own")]),
        ],
    )]));
    let stale = lib.lists().stale_playlists;
    assert_eq!(
        paths(&stale),
        [
            "Crates / Old / Sent one",
            "Crates / Old / Deeper / Sent two"
        ]
    );
    assert!(stale.iter().all(|s| s.kind == StaleKind::Playlist));
}

#[test]
fn a_path_sent_after_the_last_read_does_not_make_a_users_playlist_of_that_name_stale() {
    let lib = Lib::new();
    // The user made `Foo` in rekordbox, and a read shows it.
    lib.read_tree(crates(vec![playlist("Foo")]));
    // The app then sends a crate `Foo` and the crate is deleted before any
    // further read: the earlier read says nothing about what that send put
    // in rekordbox.
    lib.sent(&[(&["Crates", "Foo"], false)]);
    assert_eq!(lib.lists().stale_playlists, []);
    // A read after the send shows it there: now it is the app's, and stale.
    lib.read_tree(crates(vec![playlist("Foo")]));
    assert_eq!(paths(&lib.lists().stale_playlists), ["Crates / Foo"]);
}

#[test]
fn thousands_of_sibling_playlists_are_compared_in_one_pass() {
    // 5000 siblings on each side took minutes when each was compared with
    // every other. This only has to finish.
    let names: Vec<TreeNode> = (0..5000).map(|n| playlist(&format!("Crate {n}"))).collect();
    let read = RekordboxTree {
        crates: vec![folder("Big", names.clone())],
        playlists: vec![],
    };
    let app = AppTree {
        crates: vec![folder("Big", names)],
        playlists: vec![],
    };
    assert_eq!(
        stale_playlists(&read, &app, &Recorded::everything_in(&read)),
        []
    );
    // And with everything stale: one folder, listed once.
    let stale = stale_playlists(&read, &AppTree::default(), &Recorded::everything_in(&read));
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].playlists_inside, 5000);
}

impl Lib {
    /// A rekordbox row matched to `recording`, from the read made at `at`.
    fn row_for(&self, recording: i64, at: &str) {
        let at = at.to_owned();
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO rekordbox_track (attributes, location_key, read_at, recording_id)
                     VALUES ('{\"TrackID\":\"9\",\"Location\":\"file://localhost/E:/Music/elsewhere.mp3\"}', 'E:/MUSIC/ELSEWHERE.MP3', ?1, ?2)",
                    (at, recording),
                )
            })
            .unwrap();
    }
}

#[test]
fn a_removed_track_whose_sent_location_cannot_be_read_shows_its_name_and_leaves_with_its_row() {
    let lib = Lib::new();
    let recording = lib.removed(
        "Unreadable",
        Some("not a location"),
        "2026-10-01T10:00:00.000Z",
    );
    // No read since: listed, with a name and no path.
    let listed = lib.lists().manual_removals;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title.as_deref(), Some("Unreadable"));
    assert_eq!(listed[0].path, None);
    // A later read still has a row matched to the track: still listed.
    lib.read("2026-10-01T11:00:00.000Z", &[], true);
    lib.row_for(recording, "2026-10-01T11:00:00.000Z");
    assert_eq!(lib.lists().manual_removals.len(), 1);
    // A later read with no row matched to it: gone from the list.
    lib.read("2026-10-01T12:00:00.000Z", &[], true);
    assert_eq!(lib.lists().manual_removals, []);
}

#[test]
fn a_folder_holding_only_subfolders_has_no_playlists_but_is_not_empty() {
    let read = RekordboxTree {
        crates: vec![
            folder("Only folders", vec![folder("Inner", vec![])]),
            folder("Nothing", vec![]),
            playlist("A playlist"),
        ],
        playlists: vec![],
    };
    let stale = stale_playlists(&read, &AppTree::default(), &Recorded::everything_in(&read));
    assert_eq!(
        stale
            .iter()
            .map(|s| (s.path[1].as_str(), s.playlists_inside, s.empty))
            .collect::<Vec<_>>(),
        [
            ("Only folders", 0, false),
            ("Nothing", 0, true),
            ("A playlist", 0, false)
        ]
    );
}

#[test]
fn a_smart_crate_is_not_in_the_apps_tree_since_a_send_does_not_write_it() {
    let lib = Lib::new();
    lib.sent(&[(&["Crates", "Clever"], false)]);
    lib.run(
        "INSERT INTO crate (kind, name, rules) VALUES ('smart', 'Clever', '{}')",
        (),
    );
    lib.read_tree(crates(vec![playlist("Clever")]));
    // It was sent while hand-made; it isn't sent now, so rekordbox's copy is
    // one the app no longer has.
    assert_eq!(paths(&lib.lists().stale_playlists), ["Crates / Clever"]);
}

// ---- the title and artist shown (1aG-9) --------------------------------------

impl Lib {
    /// A removed track with no title or artist, sent to `location`.
    fn removed_untitled(&self, location: &str, at: &str) -> i64 {
        let recording = self.run("INSERT INTO recording DEFAULT VALUES", ());
        self.run(
            "INSERT INTO library_removal (recording_id, removed_at, last_sent_location, last_exported_at)
             VALUES (?1, ?2, ?3, '2026-09-30T10:00:00.000Z')",
            (recording, at.to_owned(), location.to_owned()),
        );
        recording
    }
}

#[test]
fn a_removed_track_shows_the_name_rekordbox_has_at_its_sent_location() {
    let lib = Lib::new();
    lib.removed_untitled(A, "2026-10-01T10:00:00.000Z");
    // A read that holds the track under a name of rekordbox's own.
    lib.read("2026-10-01T11:00:00.000Z", &[], true);
    lib.run(
        "INSERT INTO rekordbox_track (attributes, location_key, read_at)
         VALUES (?1, ?2, '2026-10-01T11:00:00.000Z')",
        (
            format!(
                "{{\"TrackID\":\"1\",\"Name\":\"Their title\",\"Artist\":\"Their artist\",\"Location\":\"{A}\"}}"
            ),
            crate::rekordbox::location::decode(A).unwrap().match_key(),
        ),
    );
    let listed = lib.lists().manual_removals;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title.as_deref(), Some("Their title"));
    assert_eq!(listed[0].artist.as_deref(), Some("Their artist"));
}

#[test]
fn a_removed_track_with_no_name_anywhere_shows_the_name_of_the_file_it_was_sent_to() {
    let lib = Lib::new();
    lib.removed_untitled(A, "2026-10-01T10:00:00.000Z");
    let listed = lib.lists().manual_removals;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title.as_deref(), Some("Synthetic A.mp3"));
    assert_eq!(listed[0].artist, None);
}
