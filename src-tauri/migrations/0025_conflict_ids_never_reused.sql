-- 0025: a `conflict` row id is never used twice (ROADMAP §2 `operation` /
-- `change`, multi-step undo; 1.10).
--
-- The operation log names a row by its id. Detecting conflicts isn't an
-- operation (it's derived from the three sides, and Undo must never take a
-- detected conflict away), and it inserts `conflict` rows; without
-- AUTOINCREMENT a new row takes the highest id plus one, which can be the
-- id of a row that removing a Library track, or resolving a conflict,
-- deleted. Undoing that operation would then find "its" row back, as
-- another conflict. With AUTOINCREMENT a new row's id is higher than any
-- the table has ever held, so a deleted row's id stays free for undo to
-- put the row back. The same change `sync_base` got in 0018.
--
-- Same columns, checks, indexes and foreign key as before (0004), and the
-- two triggers of 0014, which went with the old table; every row keeps its
-- id.
--
-- "Ever held" has to include rows deleted before this migration ran: a
-- track removed earlier still has its conflicts' ids in the operation log,
-- waiting for an undo. So the count SQLite keeps for the table
-- (`sqlite_sequence`) is raised past every `conflict` id the log names.
CREATE TABLE conflict_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    library_track_id INTEGER NOT NULL REFERENCES library_track (id) ON DELETE RESTRICT,
    field            TEXT    NOT NULL CHECK (field <> ''),
    app_value        TEXT,
    rekordbox_value  TEXT,
    -- The sync_base value both sides moved away from; NULL when the field
    -- was never sent.
    base_value       TEXT,
    status           TEXT    NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'kept_app', 'kept_rekordbox', 'edited')),
    -- The value chosen when status is `edited`.
    resolved_value   TEXT,
    detected_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    resolved_at      TEXT,
    -- Both sides changed, to different values; otherwise it isn't a
    -- conflict (1.10).
    CHECK (app_value IS NOT base_value AND rekordbox_value IS NOT base_value
           AND app_value IS NOT rekordbox_value),
    CHECK ((status = 'open') = (resolved_at IS NULL)),
    CHECK ((status = 'edited') = (resolved_value IS NOT NULL))
) STRICT;

INSERT INTO conflict_new
    (id, library_track_id, field, app_value, rekordbox_value, base_value, status,
     resolved_value, detected_at, resolved_at)
SELECT id, library_track_id, field, app_value, rekordbox_value, base_value, status,
       resolved_value, detected_at, resolved_at
FROM conflict
ORDER BY id;

DROP TABLE conflict;
ALTER TABLE conflict_new RENAME TO conflict;

CREATE INDEX conflict_by_library_track ON conflict (library_track_id);
-- One open conflict per track and field.
CREATE UNIQUE INDEX conflict_one_open_per_field ON conflict (library_track_id, field)
    WHERE status = 'open';

-- Analysis fields never become conflicts (0014).
CREATE TRIGGER conflict_never_on_an_analysis_field_insert
BEFORE INSERT ON conflict
WHEN new.field IN ('AverageBpm', 'Tonality', 'TEMPO', 'POSITION_MARK')
BEGIN
    SELECT RAISE(ABORT, 'analysis fields never become conflicts');
END;

CREATE TRIGGER conflict_never_on_an_analysis_field_update
BEFORE UPDATE OF field ON conflict
WHEN new.field IN ('AverageBpm', 'Tonality', 'TEMPO', 'POSITION_MARK')
BEGIN
    SELECT RAISE(ABORT, 'analysis fields never become conflicts');
END;

-- The copy made the table's row here only if it copied a row.
INSERT INTO sqlite_sequence (name, seq)
SELECT 'conflict', 0
WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'conflict');

UPDATE sqlite_sequence
SET seq = max(seq, (SELECT ifnull(max(entity_id), 0) FROM change WHERE entity = 'conflict'))
WHERE name = 'conflict';
