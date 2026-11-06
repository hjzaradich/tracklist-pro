//! What the screens read: the tree with its track counts, every genre
//! spelling with the node and rule it resolves to, and Review's "unmapped
//! genre" group. Files with no genre are the first queue.
//!
//! Spellings are resolved live against the current tree, so a new alias
//! empties the queue at once, without waiting for the index job.

use rusqlite::Connection;
use serde::Serialize;
use specta::Type;

use super::{aliases, nodes, AliasKind, Catalog, Resolution};
use crate::search::flag;
use crate::send_values::{shown_recordings, Shown};

/// How many tracks without a genre Review shows by name.
const NO_GENRE_SAMPLE: i64 = 25;

/// A genre, with the Library tracks the index maps to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GenreNode {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub tracks: u32,
}

/// An alias of a genre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GenreAlias {
    pub id: i64,
    pub genre_id: i64,
    pub alias: String,
    pub kind: AliasKind,
}

/// The whole taxonomy, nodes in tree order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GenreTree {
    pub nodes: Vec<GenreNode>,
    pub aliases: Vec<GenreAlias>,
}

/// A genre spelling as tracks have it, and where it goes. `resolution` is
/// `None` for an unmapped spelling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Spelling {
    pub spelling: String,
    pub tracks: u32,
    pub resolution: Option<Resolution>,
}

/// A Library track that has no genre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NoGenreTrack {
    pub recording_id: i64,
    pub title: String,
    pub artist: String,
}

/// Review's group: tracks with no genre first, then the spellings no rule
/// maps, most tracks first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UnmappedGenres {
    pub no_genre: u32,
    pub no_genre_sample: Vec<NoGenreTrack>,
    pub spellings: Vec<Spelling>,
}

fn count(n: i64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The tree with its aliases and the tracks per node.
pub fn tree(conn: &Connection) -> rusqlite::Result<GenreTree> {
    let mut per_node = std::collections::HashMap::new();
    let mut stmt = conn.prepare_cached(
        "SELECT genre_node, count(*) FROM track_row
         WHERE genre_node IS NOT NULL AND (flags & ?1) <> 0
         GROUP BY genre_node",
    )?;
    for row in stmt.query_map([flag::IN_LIBRARY], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
    })? {
        let (node, n) = row?;
        per_node.insert(node, count(n));
    }
    Ok(GenreTree {
        nodes: nodes(conn)?
            .into_iter()
            .map(|n| GenreNode {
                tracks: per_node.get(&n.id).copied().unwrap_or(0),
                id: n.id,
                parent_id: n.parent_id,
                name: n.name,
            })
            .collect(),
        aliases: conn
            .prepare_cached("SELECT id, genre_id, alias, kind FROM genre_alias ORDER BY id")?
            .query_map([], |r| {
                Ok(GenreAlias {
                    id: r.get(0)?,
                    genre_id: r.get(1)?,
                    alias: r.get(2)?,
                    kind: match r.get::<_, String>(3)?.as_str() {
                        "exact" => AliasKind::Exact,
                        _ => AliasKind::Folded,
                    },
                })
            })?
            .collect::<rusqlite::Result<_>>()?,
    })
}

/// Every spelling the Library's tracks use, most tracks first, each with
/// what the current tree makes of it.
pub fn spellings(conn: &Connection) -> rusqlite::Result<Vec<Spelling>> {
    let catalog = Catalog::new(&nodes(conn)?, &aliases(conn)?);
    let mut stmt = conn.prepare_cached(
        "SELECT genre, count(*) FROM track_row
         WHERE genre IS NOT NULL AND (flags & ?1) <> 0
         GROUP BY genre
         ORDER BY count(*) DESC, genre",
    )?;
    let rows = stmt.query_map([flag::IN_LIBRARY], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    rows.map(|row| {
        let (spelling, n) = row?;
        Ok(Spelling {
            resolution: catalog.resolve(&spelling),
            tracks: count(n),
            spelling,
        })
    })
    .collect()
}

/// Review's group.
pub fn unmapped(conn: &Connection) -> rusqlite::Result<UnmappedGenres> {
    let no_genre: i64 = conn
        .prepare_cached("SELECT count(*) FROM track_row WHERE genre IS NULL AND (flags & ?1) <> 0")?
        .query_row([flag::IN_LIBRARY], |r| r.get(0))?;
    let ids: Vec<i64> = conn
        .prepare_cached(
            "SELECT recording_id FROM track_row
             WHERE genre IS NULL AND (flags & ?1) <> 0
             ORDER BY title_key, artist_key, recording_id
             LIMIT ?2",
        )?
        .query_map(rusqlite::params![flag::IN_LIBRARY, NO_GENRE_SAMPLE], |r| {
            r.get(0)
        })?
        .collect::<rusqlite::Result<_>>()?;
    // The title and artist the lists show (the search index keeps no text).
    let mut shown = shown_recordings(conn, &ids)?;
    let no_genre_sample = ids
        .into_iter()
        .map(|recording_id| {
            let (title, artist) = shown
                .remove(&recording_id)
                .map(Shown::into_options)
                .unwrap_or_default();
            NoGenreTrack {
                recording_id,
                title: title.unwrap_or_default(),
                artist: artist.unwrap_or_default(),
            }
        })
        .collect();
    Ok(UnmappedGenres {
        no_genre: count(no_genre),
        no_genre_sample,
        spellings: spellings(conn)?
            .into_iter()
            .filter(|s| s.resolution.is_none())
            .collect(),
    })
}
