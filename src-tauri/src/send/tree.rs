//! The crate tree as the XML writer takes it.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::library::LibraryTrackId;
use crate::rekordbox_write::Node;

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
