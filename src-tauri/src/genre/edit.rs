//! Edits of the tree and its aliases. Each is one operation in the log, so
//! it can be undone; every refusal comes before the first write. User text
//! only ever goes in as a bound value.

use rusqlite::Connection;
use serde_json::json;

use super::{fold, seed, AliasKind};
use crate::ops::{self, OpsError, Recorder};

pub const ADD_OPERATION: &str = "add_genre";
pub const RENAME_OPERATION: &str = "rename_genre";
pub const MOVE_OPERATION: &str = "move_genre";
pub const ADD_ALIAS_OPERATION: &str = "add_genre_alias";
pub const REMOVE_ALIAS_OPERATION: &str = "remove_genre_alias";
pub const SEED_OPERATION: &str = "load_genre_seed";

/// The most characters a genre name or an alias may have.
pub const MAX_TEXT_CHARS: usize = 200;

/// Why an edit was refused. Nothing was changed.
#[derive(Debug)]
pub enum GenreError {
    /// There is no such genre or alias.
    NotFound,
    /// The name is empty once trimmed.
    NameEmpty,
    /// A genre already has this name (or one that differs by case or
    /// accents); `existing` is that genre's name.
    NameTaken { existing: String },
    /// A genre can't go inside itself or below its own descendants.
    MoveIntoItself,
    /// The spelling is already a genre's name or another alias; `genre` is
    /// the name of the genre it goes to.
    AliasTaken { genre: String },
    /// A new or renamed genre's name is already an alias; `genre` is the
    /// name of the genre that alias goes to.
    NameIsAlias { genre: String },
    /// The text is longer than [`MAX_TEXT_CHARS`].
    TextTooLong,
    /// The database or the log failed.
    Ops(OpsError),
}

impl From<OpsError> for GenreError {
    fn from(e: OpsError) -> Self {
        GenreError::Ops(e)
    }
}

/// An edit's answer: the refusal, or what it did.
pub type Done<T> = Result<T, GenreError>;

fn exists(rec: &Recorder<'_>, id: i64) -> Result<bool, OpsError> {
    Ok(!rec
        .read_rows("SELECT 1 FROM genre WHERE id = ?1", [id], |r| {
            r.get::<_, i64>(0)
        })?
        .is_empty())
}

/// The folded key of a new name, if the name is usable. `except` is the
/// genre being renamed (it may keep its own key).
fn checked_name(rec: &Recorder<'_>, name: &str, except: i64) -> Result<Done<String>, OpsError> {
    let key = fold(name);
    if key.is_empty() {
        return Ok(Err(GenreError::NameEmpty));
    }
    if name.chars().count() > MAX_TEXT_CHARS {
        return Ok(Err(GenreError::TextTooLong));
    }
    if let Some(genre) = rec
        .read_rows(
            "SELECT g.name FROM genre_alias a JOIN genre g ON g.id = a.genre_id
             WHERE a.alias_key = ?1",
            [&key],
            |r| r.get::<_, String>(0),
        )?
        .pop()
    {
        return Ok(Err(GenreError::NameIsAlias { genre }));
    }
    let existing = rec
        .read_rows(
            "SELECT name FROM genre WHERE name_key = ?1 AND id <> ?2",
            rusqlite::params![key, except],
            |r| r.get(0),
        )?
        .pop();
    Ok(match existing {
        Some(existing) => Err(GenreError::NameTaken { existing }),
        None => Ok(key),
    })
}

fn next_position(rec: &Recorder<'_>, parent: Option<i64>) -> Result<i64, OpsError> {
    Ok(rec
        .read_rows(
            "SELECT ifnull(max(position), -1) + 1 FROM genre WHERE parent_id IS ?1",
            [parent],
            |r| r.get(0),
        )?
        .pop()
        .unwrap_or(0))
}

/// Adds a genre under `parent` (`None`: a branch of its own), last among its
/// siblings. Names are unique across the whole tree, ignoring case and
/// accents, so a spelling can lead to only one node.
pub fn add_node(
    conn: &mut Connection,
    parent: Option<i64>,
    name: &str,
) -> Result<Done<i64>, OpsError> {
    let name = name.trim().to_owned();
    ops::record(
        conn,
        ADD_OPERATION,
        &json!({ "name": name, "parentId": parent }),
        |rec| insert_node(rec, parent, &name),
    )
    .map(|recorded| recorded.value)
}

fn insert_node(
    rec: &mut Recorder<'_>,
    parent: Option<i64>,
    name: &str,
) -> Result<Done<i64>, OpsError> {
    if let Some(parent) = parent {
        if !exists(rec, parent)? {
            return Ok(Err(GenreError::NotFound));
        }
    }
    let key = match checked_name(rec, name, -1)? {
        Ok(key) => key,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let position = next_position(rec, parent)?;
    let id = rec.insert(
        "genre",
        &[
            ("parent_id", parent.into()),
            ("name", name.to_owned().into()),
            ("name_key", key.into()),
            ("position", position.into()),
        ],
    )?;
    Ok(Ok(id))
}

/// Renames a genre. Answers the operation recorded, or `None` when the name
/// is what it was.
pub fn rename_node(
    conn: &mut Connection,
    id: i64,
    name: &str,
) -> Result<Done<Option<i64>>, OpsError> {
    let name = name.trim().to_owned();
    ops::record(
        conn,
        RENAME_OPERATION,
        &json!({ "genreId": id, "name": name }),
        |rec| {
            let Some(from) = rec
                .read_rows("SELECT name FROM genre WHERE id = ?1", [id], |r| {
                    r.get::<_, String>(0)
                })?
                .pop()
            else {
                return Ok(Err(GenreError::NotFound));
            };
            let key = match checked_name(rec, &name, id)? {
                Ok(key) => key,
                Err(refusal) => return Ok(Err(refusal)),
            };
            rec.detail("from", from);
            rec.set("genre", id, "name", name.clone())?;
            rec.set("genre", id, "name_key", key)?;
            Ok(Ok(()))
        },
    )
    .map(|recorded| recorded.value.map(|()| recorded.operation_id))
}

/// Moves a genre, with everything below it, under `parent` (`None`: to the
/// top), last among its siblings. Answers the operation recorded, or `None`
/// when it is there already.
pub fn move_node(
    conn: &mut Connection,
    id: i64,
    parent: Option<i64>,
) -> Result<Done<Option<i64>>, OpsError> {
    ops::record(
        conn,
        MOVE_OPERATION,
        &json!({ "genreId": id, "parentId": parent }),
        |rec| {
            let Some((name, from)) = rec
                .read_rows(
                    "SELECT name, parent_id FROM genre WHERE id = ?1",
                    [id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?)),
                )?
                .pop()
            else {
                return Ok(Err(GenreError::NotFound));
            };
            if let Some(target) = parent {
                if !exists(rec, target)? {
                    return Ok(Err(GenreError::NotFound));
                }
                let inside = rec
                    .read_rows(
                        "WITH RECURSIVE down (id) AS (
                             SELECT ?1
                             UNION ALL
                             SELECT g.id FROM genre g JOIN down ON g.parent_id = down.id
                         )
                         SELECT EXISTS (SELECT 1 FROM down WHERE id = ?2)",
                        (id, target),
                        |r| r.get::<_, bool>(0),
                    )?
                    .pop()
                    .unwrap_or(false);
                if inside {
                    return Ok(Err(GenreError::MoveIntoItself));
                }
            }
            if parent == from {
                return Ok(Ok(()));
            }
            let position = next_position(rec, parent)?;
            rec.detail("name", name);
            rec.set("genre", id, "parent_id", parent)?;
            rec.set("genre", id, "position", position)?;
            Ok(Ok(()))
        },
    )
    .map(|recorded| recorded.value.map(|()| recorded.operation_id))
}

/// The genre `key` already leads to: a genre's name, or an alias's genre.
fn taken_by(rec: &Recorder<'_>, key: &str) -> Result<Option<String>, OpsError> {
    Ok(rec
        .read_rows(
            "SELECT name FROM genre WHERE name_key = ?1
             UNION ALL
             SELECT g.name FROM genre_alias a JOIN genre g ON g.id = a.genre_id
             WHERE a.alias_key = ?1",
            [key],
            |r| r.get(0),
        )?
        .into_iter()
        .next())
}

/// Makes `alias` lead to genre `genre`. A spelling that is already a name or
/// an alias is refused: it would never reach this one.
pub fn add_alias(
    conn: &mut Connection,
    genre: i64,
    alias: &str,
    kind: AliasKind,
) -> Result<Done<i64>, OpsError> {
    let alias = alias.trim().to_owned();
    ops::record(
        conn,
        ADD_ALIAS_OPERATION,
        &json!({ "genreId": genre, "alias": alias }),
        |rec| insert_alias(rec, genre, &alias, kind),
    )
    .map(|recorded| recorded.value)
}

fn insert_alias(
    rec: &mut Recorder<'_>,
    genre: i64,
    alias: &str,
    kind: AliasKind,
) -> Result<Done<i64>, OpsError> {
    let Some(name) = rec
        .read_rows("SELECT name FROM genre WHERE id = ?1", [genre], |r| {
            r.get::<_, String>(0)
        })?
        .pop()
    else {
        return Ok(Err(GenreError::NotFound));
    };
    let key = fold(alias);
    if key.is_empty() {
        return Ok(Err(GenreError::NameEmpty));
    }
    if alias.chars().count() > MAX_TEXT_CHARS {
        return Ok(Err(GenreError::TextTooLong));
    }
    if let Some(genre) = taken_by(rec, &key)? {
        return Ok(Err(GenreError::AliasTaken { genre }));
    }
    rec.detail("genre", name);
    let id = rec.insert(
        "genre_alias",
        &[
            ("genre_id", genre.into()),
            ("alias", alias.to_owned().into()),
            ("alias_key", key.into()),
            ("kind", kind.as_str().to_owned().into()),
        ],
    )?;
    Ok(Ok(id))
}

/// Removes an alias. Answers the operation recorded.
pub fn remove_alias(conn: &mut Connection, id: i64) -> Result<Done<Option<i64>>, OpsError> {
    ops::record(
        conn,
        REMOVE_ALIAS_OPERATION,
        &json!({ "aliasId": id }),
        |rec| {
            let Some(alias) = rec
                .read_rows("SELECT alias FROM genre_alias WHERE id = ?1", [id], |r| {
                    r.get::<_, String>(0)
                })?
                .pop()
            else {
                return Ok(Err(GenreError::NotFound));
            };
            rec.detail("alias", alias);
            rec.delete("genre_alias", id)?;
            Ok(Ok(()))
        },
    )
    .map(|recorded| recorded.value.map(|()| recorded.operation_id))
}

/// The id of the genre named `name`, if there is one.
fn id_named(rec: &Recorder<'_>, name: &str) -> Result<Option<i64>, OpsError> {
    Ok(rec
        .read_rows(
            "SELECT id FROM genre WHERE name_key = ?1",
            [fold(name)],
            |r| r.get(0),
        )?
        .pop())
}

/// Adds the proposed starter tree ([`seed`]) as one operation: every genre
/// and alias it names that isn't there yet. Answers how many genres it
/// added.
pub fn load_seed(conn: &mut Connection) -> Result<Done<u32>, OpsError> {
    ops::record(conn, SEED_OPERATION, &json!({}), |rec| {
        let mut added = 0u32;
        for (parent, name) in seed::nodes() {
            let parent = match parent {
                Some(p) => id_named(rec, p)?,
                None => None,
            };
            if insert_node(rec, parent, name)?.is_ok() {
                added += 1;
            }
        }
        for (alias, node) in seed::aliases() {
            if let Some(id) = id_named(rec, node)? {
                // Already a name or an alias: left as it is.
                let _ = insert_alias(rec, id, alias, AliasKind::Folded)?;
            }
        }
        rec.detail("added", added);
        Ok(Ok(added))
    })
    .map(|recorded| recorded.value)
}
