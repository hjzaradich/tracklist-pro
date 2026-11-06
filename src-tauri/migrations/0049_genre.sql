-- 0050: the genre taxonomy (ROADMAP 3.2): `genre` (a tree you control),
-- `genre_alias` (a spelling that maps to a node), and the canonical node in
-- `track_row`.
--
-- Written only through the operation log, so each edit can be undone; ids
-- are never reused because nothing outside the log inserts here.
--
-- `name_key` and `alias_key` are the folded spelling (case and accents
-- dropped, `library::sort_key`), so one name can't appear twice in the tree
-- and one alias can't go to two nodes. The app keeps them equal to the
-- spelling they come from.
--
-- RESTRICT on both keys: a node with children or aliases can't be deleted
-- under the log's feet.

CREATE TABLE genre (
    id        INTEGER PRIMARY KEY,
    parent_id INTEGER REFERENCES genre (id) ON DELETE RESTRICT ON UPDATE RESTRICT,
    name      TEXT    NOT NULL CHECK (name <> ''),
    name_key  TEXT    NOT NULL UNIQUE CHECK (name_key <> ''),
    position  INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX genre_parent ON genre (parent_id, position);

CREATE TABLE genre_alias (
    id        INTEGER PRIMARY KEY,
    genre_id  INTEGER NOT NULL REFERENCES genre (id) ON DELETE RESTRICT ON UPDATE RESTRICT,
    alias     TEXT    NOT NULL CHECK (alias <> ''),
    alias_key TEXT    NOT NULL UNIQUE CHECK (alias_key <> ''),
    -- How a spelling is compared with the alias: the same text, or the same
    -- once case and accents are folded.
    kind      TEXT    NOT NULL DEFAULT 'folded' CHECK (kind IN ('exact', 'folded'))
) STRICT;

CREATE INDEX genre_alias_genre ON genre_alias (genre_id);

-- The canonical node of the track's genre spelling, kept by the index job.
-- No foreign key: it is derived, and the job rewrites it.
ALTER TABLE track_row ADD COLUMN genre_node INTEGER;

CREATE INDEX track_row_genre_node ON track_row (genre_node) WHERE genre_node IS NOT NULL;

-- Any edit of the tree or the aliases, undo included, marks every Library
-- track due (-1 stands for all of them), so `genre_node` follows.
CREATE TRIGGER genre_marks_due_insert AFTER INSERT ON genre
BEGIN INSERT OR IGNORE INTO search_due (recording_id) VALUES (-1); END;
CREATE TRIGGER genre_marks_due_update AFTER UPDATE ON genre
BEGIN INSERT OR IGNORE INTO search_due (recording_id) VALUES (-1); END;
CREATE TRIGGER genre_marks_due_delete AFTER DELETE ON genre
BEGIN INSERT OR IGNORE INTO search_due (recording_id) VALUES (-1); END;
CREATE TRIGGER genre_alias_marks_due_insert AFTER INSERT ON genre_alias
BEGIN INSERT OR IGNORE INTO search_due (recording_id) VALUES (-1); END;
CREATE TRIGGER genre_alias_marks_due_update AFTER UPDATE ON genre_alias
BEGIN INSERT OR IGNORE INTO search_due (recording_id) VALUES (-1); END;
CREATE TRIGGER genre_alias_marks_due_delete AFTER DELETE ON genre_alias
BEGIN INSERT OR IGNORE INTO search_due (recording_id) VALUES (-1); END;

INSERT OR IGNORE INTO search_due (recording_id) SELECT recording_id FROM library_track;
