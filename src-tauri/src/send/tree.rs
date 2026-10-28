//! The crate tree as the XML writer takes it.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::conflicts::membership::Memberships;
use crate::library::LibraryTrackId;
use crate::rekordbox_write::{sibling_key, Node, CRATES_FOLDER};

/// The crate tree, for the `Crates` folder of a send: folders and
/// hand-made crates, siblings in their stored order, a crate's tracks in
/// the order they were added.
///
/// A smart crate isn't sent: its tracks come from rules nothing evaluates
/// until 3.3.
pub fn crate_tree(conn: &Connection) -> rusqlite::Result<Vec<Node>> {
    let mut entries: HashMap<i64, Vec<LibraryTrackId>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT crate_id, library_track_id FROM crate_entry
         WHERE kind = 'member' ORDER BY added_at, library_track_id",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (crate_id, track): (i64, i64) = row?;
        entries
            .entry(crate_id)
            .or_default()
            .push(LibraryTrackId(track));
    }

    struct Row {
        id: i64,
        folder: bool,
        name: String,
    }
    // By parent; 0 stands for the top of the tree (no row has id 0).
    let mut children: HashMap<i64, Vec<Row>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT id, ifnull(parent_id, 0), kind, name FROM crate
         WHERE kind IN ('folder', 'static') ORDER BY position, id",
    )?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(1)?,
            Row {
                id: r.get(0)?,
                folder: r.get::<_, String>(2)? == "folder",
                name: r.get(3)?,
            },
        ))
    })? {
        let (parent, row) = row?;
        children.entry(parent).or_default().push(row);
    }

    fn nodes(
        parent: i64,
        children: &mut HashMap<i64, Vec<Row>>,
        entries: &mut HashMap<i64, Vec<LibraryTrackId>>,
    ) -> Vec<Node> {
        children
            .remove(&parent)
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                if row.folder {
                    Node::Folder {
                        name: row.name,
                        children: nodes(row.id, children, entries),
                    }
                } else {
                    Node::Playlist {
                        name: row.name,
                        entries: entries.remove(&row.id).unwrap_or_default(),
                    }
                }
            })
            .collect()
    }
    Ok(nodes(0, &mut children, &mut entries))
}

/// The crate tree as the send writes it after the merge of crate membership
/// (ROADMAP 1.10; 1cB-3): each crate's playlist without the tracks the
/// merge leaves out of it, and with the ones it adds, after the crate's own
/// ([`Memberships`]: an open conflict sends rekordbox's membership).
pub fn as_merged(nodes: &mut [Node], merged: &Memberships) {
    fn walk(nodes: &mut [Node], path: &mut Vec<String>, merged: &Memberships) {
        for node in nodes {
            match node {
                Node::Folder { name, children } => {
                    path.push(sibling_key(name));
                    walk(children, path, merged);
                    path.pop();
                }
                Node::Playlist { name, entries } => {
                    path.push(sibling_key(name));
                    if let Some(dropped) = merged.dropped.get(path.as_slice()) {
                        entries.retain(|track| !dropped.contains(track));
                    }
                    for track in merged.added.get(path.as_slice()).into_iter().flatten() {
                        if !entries.contains(track) {
                            entries.push(*track);
                        }
                    }
                    path.pop();
                }
            }
        }
    }
    walk(nodes, &mut vec![sibling_key(CRATES_FOLDER)], merged);
}
