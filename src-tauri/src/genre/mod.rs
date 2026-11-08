//! The genre taxonomy (ROADMAP 3.2; migration 0049): a tree of genres you
//! control, and aliases that map the mess of real spellings onto its nodes.
//!
//! - The resolver here is pure: [`Catalog::resolve`] takes a spelling and
//!   answers a node and the [`Rule`] that applied, or nothing (the spelling
//!   is "unmapped" and goes to Review).
//! - A track keeps its own spelling; the canonical node is only an answer
//!   next to it (`track_row.genre_node`, kept by the index job). The send
//!   writes rekordbox's spelling as it was: the taxonomy is app-side only.
//! - Genre belongs to the track (a recording), not the song: a VIP of a
//!   house tune keeps its own genre.
//! - Edits are operations in the log ([`edit`]), so each can be undone.
//! - The starter tree ([`seed`]) is a proposal, loaded only when asked for.

pub mod commands;
pub mod edit;
pub mod own;
pub mod seed;
pub mod unmapped;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::library::sort_key;

/// Text as the taxonomy compares it when case and accents don't matter.
pub fn fold(text: &str) -> String {
    sort_key(text.trim())
}

/// Which rule mapped a spelling to a node. Tried in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Rule {
    /// The spelling is the node's name, letter for letter.
    Exact,
    /// The same once case and accents are folded.
    Folded,
    /// `Parent (Sub)`: a node `Sub` that sits under a node `Parent`.
    ParentSub,
    /// An alias the user made.
    Alias,
}

/// How an alias is compared with a spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AliasKind {
    /// The same text.
    Exact,
    /// The same once case and accents are folded.
    Folded,
}

impl AliasKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AliasKind::Exact => "exact",
            AliasKind::Folded => "folded",
        }
    }

    fn parse(text: &str) -> AliasKind {
        match text {
            "exact" => AliasKind::Exact,
            _ => AliasKind::Folded,
        }
    }
}

/// A node of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
}

/// A spelling that goes to a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    pub genre_id: i64,
    pub alias: String,
    pub kind: AliasKind,
}

/// The node a spelling resolved to, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub node: i64,
    pub rule: Rule,
}

/// The tree and the aliases, indexed to resolve spellings.
#[derive(Debug, Default)]
pub struct Catalog {
    by_name: HashMap<String, i64>,
    by_key: HashMap<String, i64>,
    parents: HashMap<i64, Option<i64>>,
    exact_aliases: HashMap<String, i64>,
    folded_aliases: HashMap<String, i64>,
}

impl Catalog {
    pub fn new(nodes: &[Node], aliases: &[Alias]) -> Catalog {
        let mut c = Catalog::default();
        for n in nodes {
            c.by_name.insert(n.name.clone(), n.id);
            c.by_key.insert(fold(&n.name), n.id);
            c.parents.insert(n.id, n.parent_id);
        }
        for a in aliases {
            match a.kind {
                AliasKind::Exact => c
                    .exact_aliases
                    .insert(a.alias.trim().to_owned(), a.genre_id),
                AliasKind::Folded => c.folded_aliases.insert(fold(&a.alias), a.genre_id),
            };
        }
        c
    }

    /// Reads the tree and the aliases.
    pub fn load(conn: &Connection) -> rusqlite::Result<Catalog> {
        Ok(Catalog::new(&nodes(conn)?, &aliases(conn)?))
    }

    /// The node `spelling` belongs to and the rule that said so; `None` if
    /// no rule applies. Never guesses: a spelling that is not a node, an
    /// alias or `Parent (Sub)` of known nodes stays unmapped.
    pub fn resolve(&self, spelling: &str) -> Option<Resolution> {
        let spelling = spelling.trim();
        if spelling.is_empty() {
            return None;
        }
        let hit = |node: Option<&i64>, rule| node.map(|&node| Resolution { node, rule });
        let key = fold(spelling);
        hit(self.by_name.get(spelling), Rule::Exact)
            .or_else(|| hit(self.by_key.get(&key), Rule::Folded))
            .or_else(|| hit(self.parent_sub(spelling).as_ref(), Rule::ParentSub))
            .or_else(|| {
                hit(
                    self.exact_aliases
                        .get(spelling)
                        .or_else(|| self.folded_aliases.get(&key)),
                    Rule::Alias,
                )
            })
    }

    /// `Parent (Sub)`: the node `Sub` when it is under the node `Parent`.
    fn parent_sub(&self, spelling: &str) -> Option<i64> {
        let body = spelling.strip_suffix(')')?;
        let open = body.rfind('(')?;
        let (parent, sub) = (body[..open].trim(), body[open + 1..].trim());
        if parent.is_empty() || sub.is_empty() {
            return None;
        }
        let parent = *self.by_key.get(&fold(parent))?;
        let sub = *self.by_key.get(&fold(sub))?;
        let mut up = self.parents.get(&sub).copied().flatten();
        // The tree is finite, but a bad database must not loop forever.
        for _ in 0..self.parents.len() {
            match up {
                Some(p) if p == parent => return Some(sub),
                Some(p) => up = self.parents.get(&p).copied().flatten(),
                None => return None,
            }
        }
        None
    }
}

/// Every node, in tree order (siblings by position, then name).
pub fn nodes(conn: &Connection) -> rusqlite::Result<Vec<Node>> {
    conn.prepare_cached(
        "SELECT id, parent_id, name FROM genre ORDER BY ifnull(parent_id, 0), position, id",
    )?
    .query_map([], |r| {
        Ok(Node {
            id: r.get(0)?,
            parent_id: r.get(1)?,
            name: r.get(2)?,
        })
    })?
    .collect()
}

fn aliases(conn: &Connection) -> rusqlite::Result<Vec<Alias>> {
    conn.prepare_cached("SELECT genre_id, alias, kind FROM genre_alias ORDER BY id")?
        .query_map([], |r| {
            Ok(Alias {
                genre_id: r.get(0)?,
                alias: r.get(1)?,
                kind: AliasKind::parse(&r.get::<_, String>(2)?),
            })
        })?
        .collect()
}
