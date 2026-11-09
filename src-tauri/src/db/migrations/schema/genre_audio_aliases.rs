//! 0060: aliases from the audio model's genre names to the genre tree (ROADMAP
//! 3.8, 6B-F3). The migration runs on a library that already has genres, so
//! these tests build one at 0059 first.

use rusqlite::Connection;

use super::{accepts, db_at, one};
use crate::audio_model::subset::SUBSET;
use crate::db::migrations::{run, MIGRATIONS};
use crate::genre::seed::{self, AUDIO_MODEL_ALIASES};
use crate::genre::Catalog;

/// A library at 0059 with the whole starter tree in it, as a library that
/// loaded the seed before this migration has it (nodes only, no model aliases).
fn with_the_starter_tree() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db_at(59);
    for (parent, name) in seed::nodes() {
        let parent_id = match parent {
            Some(p) => format!(
                "(SELECT id FROM genre WHERE name = '{}')",
                p.replace('\'', "''")
            ),
            None => "NULL".to_owned(),
        };
        accepts(
            &conn,
            &format!(
                "INSERT INTO genre (parent_id, name, name_key) VALUES ({parent_id}, '{n}', lower('{n}'))",
                n = name.replace('\'', "''")
            ),
        );
    }
    (dir, conn)
}

fn migrate(conn: &mut Connection) {
    run(conn, MIGRATIONS).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
}

/// `(alias, node name)` of every alias in the library, sorted.
fn aliases(conn: &Connection) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT a.alias, g.name FROM genre_alias a JOIN genre g ON g.id = a.genre_id
             ORDER BY a.alias",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn sorted_pairs() -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = AUDIO_MODEL_ALIASES
        .iter()
        .map(|&(a, n)| (a.to_owned(), n.to_owned()))
        .collect();
    pairs.sort();
    pairs
}

#[test]
fn a_library_with_the_starter_tree_gets_exactly_the_alias_pairs_the_seed_lists() {
    let (_dir, mut conn) = with_the_starter_tree();
    assert!(aliases(&conn).is_empty());
    migrate(&mut conn);
    assert_eq!(aliases(&conn), sorted_pairs());
    // The migration's own spelling is the folded one the resolver looks up.
    let unfolded: i64 = one(
        &conn,
        "SELECT count(*) FROM genre_alias WHERE alias_key <> lower(alias) OR kind <> 'folded'",
    );
    assert_eq!(unfolded, 0);
}

#[test]
fn every_audio_model_genre_name_reaches_a_node_or_is_named_here_as_having_none() {
    // AudioSet genres with no node in the starter tree: they stay unmapped
    // until the user builds one.
    const NO_NODE: &[&str] = &[
        "Beatboxing",
        "Heavy metal",
        "Grunge",
        "Progressive rock",
        "Rock and roll",
        "Psychedelic rock",
        "Country",
        "Swing music",
        "Bluegrass",
        "Folk music",
        "Middle Eastern music",
        "Classical music",
        "Opera",
        "Flamenco",
        "Blues",
        "Music for children",
        "New-age music",
        "Vocal music",
        "A capella",
        "Music of Africa",
        "Christian music",
        "Gospel music",
        "Music of Asia",
        "Carnatic music",
        "Music of Bollywood",
        "Ska",
        "Traditional music",
        "Independent music",
    ];
    let (_dir, mut conn) = with_the_starter_tree();
    migrate(&mut conn);
    let catalog = Catalog::load(&conn).unwrap();
    // The genre labels run from "Pop music" to "Independent music".
    let genres = &SUBSET[..50];
    assert_eq!(genres[0].1, "Pop music");
    assert_eq!(genres[49].1, "Independent music");
    for (_, name) in genres {
        let reaches = catalog.resolve(name).is_some();
        assert_eq!(
            reaches,
            !NO_NODE.contains(name),
            "{name}: resolves {reaches}, listed as having no node {}",
            NO_NODE.contains(name)
        );
    }
}

#[test]
fn a_name_the_user_already_made_an_alias_keeps_its_node() {
    let (_dir, mut conn) = with_the_starter_tree();
    accepts(
        &conn,
        "INSERT INTO genre_alias (genre_id, alias, alias_key, kind)
         VALUES ((SELECT id FROM genre WHERE name = 'Techno'), 'House Music', 'house music', 'folded')",
    );
    migrate(&mut conn);
    let node: String = one(
        &conn,
        "SELECT g.name FROM genre_alias a JOIN genre g ON g.id = a.genre_id WHERE a.alias_key = 'house music'",
    );
    assert_eq!(node, "Techno", "the user's alias wins");
    let spelled: String = one(
        &conn,
        "SELECT alias FROM genre_alias WHERE alias_key = 'house music'",
    );
    assert_eq!(spelled, "House Music");
    // The rest are added as usual.
    assert_eq!(aliases(&conn).len(), AUDIO_MODEL_ALIASES.len());
}

#[test]
fn a_name_the_user_already_made_a_genre_is_not_aliased_away_from_it() {
    let (_dir, mut conn) = db_at(59);
    accepts(
        &conn,
        "INSERT INTO genre (name, name_key) VALUES ('Pop', 'pop'), ('Pop music', 'pop music');",
    );
    migrate(&mut conn);
    let aliased: i64 = one(
        &conn,
        "SELECT count(*) FROM genre_alias WHERE alias_key = 'pop music'",
    );
    assert_eq!(aliased, 0, "a node of that name stays the only answer");
    let catalog = Catalog::load(&conn).unwrap();
    let pop_music: i64 = one(&conn, "SELECT id FROM genre WHERE name_key = 'pop music'");
    assert_eq!(
        catalog.resolve("Pop music").map(|r| r.node),
        Some(pop_music)
    );
}

#[test]
fn a_library_with_no_genres_gets_no_aliases_and_nothing_fails() {
    let (_dir, mut conn) = db_at(59);
    migrate(&mut conn);
    assert!(aliases(&conn).is_empty());
}
