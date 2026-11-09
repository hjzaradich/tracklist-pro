-- 0060: aliases from the audio model's generic genre names to the genre tree
-- (ROADMAP 3.8, 6B-F3). "Use" on a guess sends its name as shown ("House",
-- "Electronic dance"), and a name that is not a node or an alias goes to
-- Unmapped. These are the names that differ from a tree node's name, each
-- pointing at its node: both the AudioSet name ("House music") and the name
-- the panel shows.
--
-- Added for a library that already has the node (it loaded the starter tree,
-- or built its own): the node is looked up by name, so a library without it
-- gets nothing here, and loading the starter tree later adds the same aliases
-- (`genre::seed`). Never overwrites the user's data: a spelling that is already
-- an alias (the unique `alias_key`) or already a node name is left as it is.
-- The same pairs are in `genre::seed::AUDIO_MODEL_ALIASES`; a test keeps the two
-- in step.

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Pop music', 'pop music', 'folded' FROM genre g
WHERE g.name_key = 'pop'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'pop music');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Hip hop music', 'hip hop music', 'folded' FROM genre g
WHERE g.name_key = 'hip-hop'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'hip hop music');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Hip hop', 'hip hop', 'folded' FROM genre g
WHERE g.name_key = 'hip-hop'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'hip hop');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Rock music', 'rock music', 'folded' FROM genre g
WHERE g.name_key = 'rock'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'rock music');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Punk rock', 'punk rock', 'folded' FROM genre g
WHERE g.name_key = 'punk'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'punk rock');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Rhythm and blues', 'rhythm and blues', 'folded' FROM genre g
WHERE g.name_key = 'r&b'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'rhythm and blues');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Soul music', 'soul music', 'folded' FROM genre g
WHERE g.name_key = 'soul'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'soul music');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Funk', 'funk', 'folded' FROM genre g
WHERE g.name_key = 'funk & disco'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'funk');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Disco', 'disco', 'folded' FROM genre g
WHERE g.name_key = 'funk & disco'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'disco');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Reggae', 'reggae', 'folded' FROM genre g
WHERE g.name_key = 'reggae & dancehall'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'reggae');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Electronic music', 'electronic music', 'folded' FROM genre g
WHERE g.name_key = 'electronic'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'electronic music');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Electronica', 'electronica', 'folded' FROM genre g
WHERE g.name_key = 'electronic'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'electronica');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Electronic dance music', 'electronic dance music', 'folded' FROM genre g
WHERE g.name_key = 'electronic'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'electronic dance music');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Electronic dance', 'electronic dance', 'folded' FROM genre g
WHERE g.name_key = 'electronic'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'electronic dance');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'House music', 'house music', 'folded' FROM genre g
WHERE g.name_key = 'house'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'house music');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Drum and bass', 'drum and bass', 'folded' FROM genre g
WHERE g.name_key = 'drum & bass'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'drum and bass');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Ambient music', 'ambient music', 'folded' FROM genre g
WHERE g.name_key = 'ambient'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'ambient music');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Trance music', 'trance music', 'folded' FROM genre g
WHERE g.name_key = 'trance'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'trance music');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Music of Latin America', 'music of latin america', 'folded' FROM genre g
WHERE g.name_key = 'latin'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'music of latin america');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Latin American', 'latin american', 'folded' FROM genre g
WHERE g.name_key = 'latin'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'latin american');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Salsa music', 'salsa music', 'folded' FROM genre g
WHERE g.name_key = 'salsa'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'salsa music');

INSERT OR IGNORE INTO genre_alias (genre_id, alias, alias_key, kind)
SELECT g.id, 'Afrobeat', 'afrobeat', 'folded' FROM genre g
WHERE g.name_key = 'afrobeats'
  AND NOT EXISTS (SELECT 1 FROM genre WHERE name_key = 'afrobeat');
