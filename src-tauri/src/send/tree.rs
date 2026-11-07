//! The crate tree as the XML writer takes it.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::conflicts::membership::Memberships;
use crate::library::LibraryTrackId;
use crate::rekordbox_write::{sibling_key, Node, CRATES_FOLDER};

/// The crate tree, for the `Crates` folder of a send: folders, hand-made
/// crates and smart crates, siblings in their stored order, a hand-made
/// crate's tracks in the order they were added. A smart crate is evaluated
/// now and goes out as a plain playlist of what it holds
/// ([`crate::crates::smart::membership`]), so each send refreshes it.
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
    entries.extend(crate::crates::smart::membership::evaluated(conn)?);

    struct Row {
        id: i64,
        folder: bool,
        name: String,
    }
    // By parent; 0 stands for the top of the tree (no row has id 0).
    let mut children: HashMap<i64, Vec<Row>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT id, ifnull(parent_id, 0), kind, name FROM crate
         ORDER BY position, id",
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

/// Everything a send writes under [`PLAYLISTS_FOLDER`]: the app's playlists
/// in position order (3.5, [`crate::playlists::send_nodes`]), then the
/// `Tags` folder ([`tag_playlists`]). A playlist's name never equals `Tags`
/// or another's by the sibling rule (the playlists module refuses it).
///
/// [`PLAYLISTS_FOLDER`]: crate::rekordbox_write::PLAYLISTS_FOLDER
pub fn playlist_tree(conn: &Connection) -> rusqlite::Result<Vec<Node>> {
    let mut nodes = crate::playlists::send_nodes(conn)?;
    nodes.extend(tag_playlists(conn)?);
    Ok(nodes)
}

/// The folder under `Playlists` that holds one playlist per tag (3A-T; T5).
pub const TAGS_FOLDER: &str = "Tags";

/// The playlists a send writes under [`PLAYLISTS_FOLDER`]: one folder,
/// `Tags`, with one playlist per tag that has Library tracks, in the order of
/// the vocabulary (groups, then tags), each playlist's tracks in the order
/// they were tagged. Library tracks only (the tracks a send carries).
///
/// Only playlists: nothing is written into a track's comment, and no My Tags
/// block (ROADMAP 1.9 rule 1; writing My Tags is untested, T5).
///
/// A tag the writer can't carry as a name, or whose name equals an earlier
/// tag's by the sibling rule, is left out, so a tag is never the reason a
/// send is refused. With no such tag there is no `Tags` folder.
pub fn tag_playlists(conn: &Connection) -> rusqlite::Result<Vec<Node>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, l.id
         FROM tag t
         JOIN tag_group g ON g.id = t.group_id
         JOIN tag_applied a ON a.tag_id = t.id
         JOIN library_track l ON l.recording_id = a.recording_id
         ORDER BY g.position, g.id, t.position, t.id, a.id",
    )?;
    let mut playlists: Vec<(i64, Node)> = Vec::new();
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })? {
        let (tag, name, track) = row?;
        match playlists.last_mut() {
            Some((last, Node::Playlist { entries, .. })) if *last == tag => {
                entries.push(LibraryTrackId(track))
            }
            _ => playlists.push((
                tag,
                Node::Playlist {
                    name,
                    entries: vec![LibraryTrackId(track)],
                },
            )),
        }
    }
    let mut taken = std::collections::HashSet::new();
    let children: Vec<Node> = playlists
        .into_iter()
        .map(|(_, node)| node)
        .filter(|node| match node {
            Node::Playlist { name, .. } => {
                crate::vocab::sendable(name) && taken.insert(sibling_key(name))
            }
            Node::Folder { .. } => false,
        })
        .collect();
    Ok(if children.is_empty() {
        Vec::new()
    } else {
        vec![Node::Folder {
            name: TAGS_FOLDER.to_owned(),
            children,
        }]
    })
}
