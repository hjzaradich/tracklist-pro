-- 0018: a `sync_base` row id is never used twice (ROADMAP §2 `operation` /
-- `change`, multi-step undo).
--
-- The operation log names a row by its id. A send isn't an operation, and
-- it inserts `sync_base` rows; without AUTOINCREMENT a new row takes the
-- highest id plus one, which can be the id of a row that removing a Library
-- track deleted. Undoing that removal would then find "its" row back, as
-- another track's base. With AUTOINCREMENT a new row's id is higher than
-- any the table has ever held, so a deleted row's id stays free for undo to
-- put the row back.
--
-- Same columns, checks, unique key and foreign key as before (0013); every
-- row keeps its id.
--
-- "Ever held" has to include rows deleted before this migration ran: a
-- track removed earlier still has its bases' ids in the operation log,
-- waiting for an undo. So the count SQLite keeps for the table
-- (`sqlite_sequence`) is raised past every `sync_base` id the log names.
CREATE TABLE sync_base_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    library_track_id INTEGER NOT NULL REFERENCES library_track (id) ON DELETE RESTRICT,
    -- The field as sent, e.g. an XML attribute name such as `Rating`.
    field            TEXT    NOT NULL CHECK (field <> ''),
    value            TEXT    NOT NULL,
    synced_at        TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- One base value per track and field.
    UNIQUE (library_track_id, field)
) STRICT;

INSERT INTO sync_base_new (id, library_track_id, field, value, synced_at)
SELECT id, library_track_id, field, value, synced_at FROM sync_base
ORDER BY id;

DROP TABLE sync_base;
ALTER TABLE sync_base_new RENAME TO sync_base;

-- The copy made the table's row here only if it copied a row.
INSERT INTO sqlite_sequence (name, seq)
SELECT 'sync_base', 0
WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'sync_base');

UPDATE sqlite_sequence
SET seq = max(seq, (SELECT ifnull(max(entity_id), 0) FROM change WHERE entity = 'sync_base'))
WHERE name = 'sync_base';
