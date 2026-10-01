//! Playlist names: rekordbox's, as last read, and the app's own, and what
//! is in the first and not the second.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::rekordbox::{self, RekordboxXml};
use crate::rekordbox_write::{Node, CRATES_FOLDER, PLAYLISTS_FOLDER};

/// The setting that holds the last read's playlist names ([`RekordboxTree`]).
pub const PLAYLIST_TREE: &str = "rekordbox_playlist_tree";

/// A folder or a playlist, by name only: what stale-playlist checking
/// needs, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeNode {
    pub name: String,
    /// `Some` for a folder (possibly empty), `None` for a playlist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeNode>>,
}

impl TreeNode {
    pub fn playlist(name: &str) -> TreeNode {
        TreeNode {
            name: name.to_owned(),
            children: None,
        }
    }

    pub fn folder(name: &str, children: Vec<TreeNode>) -> TreeNode {
        TreeNode {
            name: name.to_owned(),
            children: Some(children),
        }
    }

    fn playlists_inside(&self) -> u32 {
        match &self.children {
            None => 1,
            Some(children) => children.iter().map(TreeNode::playlists_inside).sum(),
        }
    }
}

impl From<&Node> for TreeNode {
    fn from(node: &Node) -> TreeNode {
        match node {
            Node::Folder { name, children } => TreeNode {
                name: name.clone(),
                children: Some(children.iter().map(TreeNode::from).collect()),
            },
            Node::Playlist { name, .. } => TreeNode::playlist(name),
        }
    }
}

impl From<&rekordbox::Node> for TreeNode {
    fn from(node: &rekordbox::Node) -> TreeNode {
        match node {
            rekordbox::Node::Folder(folder) => TreeNode {
                name: folder.name.clone(),
                children: Some(folder.children.iter().map(TreeNode::from).collect()),
            },
            rekordbox::Node::Playlist(playlist) => TreeNode::playlist(&playlist.name),
        }
    }
}

/// What lies under the `Crates` and `Playlists` folders in rekordbox, and
/// nothing outside them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RekordboxTree {
    pub crates: Vec<TreeNode>,
    pub playlists: Vec<TreeNode>,
}

/// What the app's own tree holds, for the same two folders.
pub type AppTree = RekordboxTree;

/// What two sibling names are compared by: the writer's rule (NFC, letter
/// case and trailing whitespace ignored; ROADMAP §5.2).
fn key(name: &str) -> String {
    crate::rekordbox_write::sibling_key(name)
}

impl RekordboxTree {
    /// The two folders' contents from a parsed export. A top-level folder
    /// rekordbox doesn't have is empty here.
    pub fn from_xml(xml: &RekordboxXml) -> RekordboxTree {
        let under = |top: &str| -> Vec<TreeNode> {
            xml.playlists
                .children
                .iter()
                .filter_map(|node| match node {
                    rekordbox::Node::Folder(folder) if key(&folder.name) == key(top) => {
                        Some(folder.children.iter().map(TreeNode::from))
                    }
                    _ => None,
                })
                .flatten()
                .collect()
        };
        RekordboxTree {
            crates: under(CRATES_FOLDER),
            playlists: under(PLAYLISTS_FOLDER),
        }
    }

    /// The app's tree from the trees a send is built from.
    pub fn from_nodes(crates: &[Node], playlists: &[Node]) -> RekordboxTree {
        RekordboxTree {
            crates: crates.iter().map(TreeNode::from).collect(),
            playlists: playlists.iter().map(TreeNode::from).collect(),
        }
    }
}

/// Keeps the last read's names, replacing the earlier ones. Call it inside
/// the read's own transaction.
pub fn save_tree(conn: &Connection, tree: &RekordboxTree) -> rusqlite::Result<()> {
    let json = serde_json::to_string(tree)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET
             value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (PLAYLIST_TREE, json),
    )?;
    settle_recorded(conn, tree)
}

/// The last read's names, or `None` if no read has kept them.
pub fn load_tree(conn: &Connection) -> rusqlite::Result<Option<RekordboxTree>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT value FROM setting WHERE key = ?1",
            [PLAYLIST_TREE],
            |r| r.get(0),
        )
        .optional()?;
    Ok(json.and_then(|json| serde_json::from_str(&json).ok()))
}

/// Whether a stale entry is a folder or a playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum StaleKind {
    Folder,
    Playlist,
}

/// A playlist or folder in rekordbox that the app doesn't have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StalePlaylist {
    /// Folder names down to it, starting with `Crates` or `Playlists`.
    pub path: Vec<String>,
    pub kind: StaleKind,
    /// For a folder, how many playlists are in it (at any depth).
    pub playlists_inside: u32,
    /// A folder with nothing in it at all (not even a subfolder). A folder
    /// holding only subfolders has no playlists but isn't empty.
    pub empty: bool,
}

/// The app's own tree: what a send would write, from the `crate` table
/// ([`crate::send::crate_tree`]): folders and hand-made crates. A smart crate
/// isn't sent yet (3.3), so it isn't here. The app has no playlists of its
/// own yet (Phase 3).
pub fn app_tree(conn: &Connection) -> rusqlite::Result<AppTree> {
    Ok(RekordboxTree::from_nodes(
        &crate::send::crate_tree(conn)?,
        &[],
    ))
}

/// The playlists and folders a send has written, as normalized name paths
/// from the top-level folder down (`sent_playlist`, ROADMAP 1.9 rule 6).
/// Only these are ever listed as stale: what the user made in rekordbox
/// isn't here, even inside `Crates` and `Playlists`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Recorded(HashSet<(Vec<String>, StaleKind)>);

impl Recorded {
    /// A recorded path, from the names as written.
    pub fn of(path: &[&str], kind: StaleKind) -> Recorded {
        let mut recorded = Recorded::default();
        recorded.add(path, kind);
        recorded
    }

    pub fn add(&mut self, path: &[&str], kind: StaleKind) {
        self.0.insert((path.iter().map(|n| key(n)).collect(), kind));
    }

    fn has(&self, key_path: &[String], kind: StaleKind) -> bool {
        self.0.contains(&(key_path.to_vec(), kind))
    }
}

/// Every path a send has written that a read has shown in rekordbox and not
/// yet shown gone. A path recorded by a send after the last read is not
/// here: that read says nothing about what the send put there.
pub fn load_recorded(conn: &Connection) -> rusqlite::Result<Recorded> {
    let mut recorded = Recorded::default();
    let mut stmt = conn.prepare("SELECT path_key, kind FROM sent_playlist WHERE seen = 1")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (path_key, kind) = row?;
        if let Ok(path_key) = serde_json::from_str::<Vec<String>>(&path_key) {
            let kind = if kind == "folder" {
                StaleKind::Folder
            } else {
                StaleKind::Playlist
            };
            recorded.0.insert((path_key, kind));
        }
    }
    Ok(recorded)
}

impl RekordboxTree {
    /// Every folder and playlist as a normalized name path from its
    /// top-level folder, with its kind.
    fn keyed_paths(&self) -> HashSet<(Vec<String>, StaleKind)> {
        fn below(
            nodes: &[TreeNode],
            path: &mut Vec<String>,
            out: &mut HashSet<(Vec<String>, StaleKind)>,
        ) {
            for node in nodes {
                path.push(key(&node.name));
                let kind = if node.children.is_some() {
                    StaleKind::Folder
                } else {
                    StaleKind::Playlist
                };
                out.insert((path.clone(), kind));
                if let Some(children) = &node.children {
                    below(children, path, out);
                }
                path.pop();
            }
        }
        let mut out = HashSet::new();
        below(&self.crates, &mut vec![key(CRATES_FOLDER)], &mut out);
        below(&self.playlists, &mut vec![key(PLAYLISTS_FOLDER)], &mut out);
        out
    }
}

/// Brings the recorded sends in step with a read. A recorded path the read
/// holds is marked seen; one it lacks, that an earlier read had shown, is
/// forgotten (the user deleted it). A path never seen stays: that send may
/// not have been imported yet. Call it inside the read's own transaction.
fn settle_recorded(conn: &Connection, tree: &RekordboxTree) -> rusqlite::Result<()> {
    let present = tree.keyed_paths();
    let rows: Vec<(i64, String, String, bool)> = conn
        .prepare("SELECT id, path_key, kind, seen FROM sent_playlist")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, path_key, kind, seen) in rows {
        let kind = if kind == "folder" {
            StaleKind::Folder
        } else {
            StaleKind::Playlist
        };
        let here = serde_json::from_str::<Vec<String>>(&path_key)
            .is_ok_and(|path_key| present.contains(&(path_key, kind)));
        if here && !seen {
            conn.execute("UPDATE sent_playlist SET seen = 1 WHERE id = ?1", [id])?;
        } else if !here && seen {
            conn.execute("DELETE FROM sent_playlist WHERE id = ?1", [id])?;
        }
    }
    Ok(())
}

/// What rekordbox has under `Crates` and `Playlists` that the app sent
/// before and no longer has. A playlist or folder is listed only if
/// `recorded` says a send wrote it: something the user made in rekordbox is
/// never listed. Matching is by name (see [`key`]) and kind: a folder never
/// stands for a playlist of the same name.
///
/// A folder the app no longer has is listed, once and without its contents,
/// only when everything rekordbox has inside it (at any depth) was sent by
/// the app too: deleting it then deletes only what the app sent, and its
/// count is exactly that. If it holds anything the user made, the folder is
/// not listed; the recorded playlists and folders inside it are, one by one.
/// A folder the app has, or one the user made, is looked into.
pub fn stale_playlists(
    read: &RekordboxTree,
    app: &AppTree,
    recorded: &Recorded,
) -> Vec<StalePlaylist> {
    let mut out = Vec::new();
    for (top, read, app) in [
        (CRATES_FOLDER, &read.crates, &app.crates),
        (PLAYLISTS_FOLDER, &read.playlists, &app.playlists),
    ] {
        let mut walk = Walk {
            recorded,
            path: vec![top.to_owned()],
            keys: vec![key(top)],
            out: &mut out,
        };
        walk.nodes(read, app);
    }
    out
}

struct Walk<'a> {
    recorded: &'a Recorded,
    /// The names down to the level being walked, as rekordbox has them.
    path: Vec<String>,
    /// The same, normalized.
    keys: Vec<String>,
    out: &'a mut Vec<StalePlaylist>,
}

impl Walk<'_> {
    /// `app` is the app's nodes at this level (none below a folder the app
    /// doesn't have). Each level looks the app's siblings up in a map built
    /// once, so a big folder costs one pass.
    fn nodes(&mut self, read: &[TreeNode], app: &[TreeNode]) {
        let mine: HashMap<(String, bool), &TreeNode> = app
            .iter()
            .map(|n| ((key(&n.name), n.children.is_some()), n))
            .collect();
        for node in read {
            let node_key = key(&node.name);
            let kind = if node.children.is_some() {
                StaleKind::Folder
            } else {
                StaleKind::Playlist
            };
            let found = mine.get(&(node_key.clone(), node.children.is_some()));
            self.path.push(node.name.clone());
            self.keys.push(node_key);
            match (found, &node.children) {
                // The app has it: nothing stale, but look inside a folder.
                (Some(found), Some(children)) => {
                    self.nodes(children, found.children.as_deref().unwrap_or(&[]));
                }
                (Some(_), None) => {}
                (None, children) => {
                    let sent = self.recorded.has(&self.keys, kind);
                    let whole = children.as_ref().is_none_or(|c| self.all_sent(c));
                    if sent && whole {
                        self.out.push(StalePlaylist {
                            path: self.path.clone(),
                            kind,
                            playlists_inside: if children.is_some() {
                                node.playlists_inside()
                            } else {
                                0
                            },
                            empty: children.as_ref().is_some_and(|c| c.is_empty()),
                        });
                    } else if let Some(children) = children {
                        // Not listed whole: the user's folder, or one with
                        // something of theirs inside. What the app sent
                        // inside it is still found.
                        self.nodes(children, &[]);
                    }
                }
            }
            self.path.pop();
            self.keys.pop();
        }
    }

    /// Whether everything below the folder being walked, at any depth, was
    /// sent by the app.
    fn all_sent(&mut self, children: &[TreeNode]) -> bool {
        for child in children {
            let kind = if child.children.is_some() {
                StaleKind::Folder
            } else {
                StaleKind::Playlist
            };
            self.keys.push(key(&child.name));
            let sent = self.recorded.has(&self.keys, kind)
                && child.children.as_ref().is_none_or(|c| self.all_sent(c));
            self.keys.pop();
            if !sent {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
impl Recorded {
    /// As if the app had sent everything `tree` holds.
    pub fn everything_in(tree: &RekordboxTree) -> Recorded {
        Recorded(tree.keyed_paths())
    }
}
