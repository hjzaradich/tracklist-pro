-- 0039: crate membership merged before every send (ROADMAP 1.10, §2
-- `crate_entry` and `sent_playlist`; 1cB-3).
--
-- 1. `crate_entry` ids are never used twice (AUTOINCREMENT). The merge
--    before a send takes a playlist change made in rekordbox into the
--    crate: it inserts and deletes `crate_entry` rows outside the
--    operation log, as detecting conflicts does with `conflict` rows. The
--    log names rows by id; without AUTOINCREMENT a new row can take the id
--    of one an operation recorded, and undoing that operation would act on
--    the wrong row. The table is rebuilt with every row under its own id,
--    the same columns, checks and foreign keys, and its index and triggers
--    put back as they were declared. Foreign keys are off while migrations
--    run, so dropping the old table deletes nothing that points at it.
--    Nothing outside the table refers to it by name.
CREATE TABLE crate_entry_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    crate_id         INTEGER NOT NULL REFERENCES crate (id) ON DELETE CASCADE,
    library_track_id INTEGER NOT NULL REFERENCES library_track (id) ON DELETE RESTRICT,
    kind             TEXT    NOT NULL DEFAULT 'member'
        CHECK (kind IN ('member', 'always_include', 'never_include')),
    added_at         TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A track is in a crate once.
    UNIQUE (crate_id, library_track_id)
) STRICT;

INSERT INTO crate_entry_new (id, crate_id, library_track_id, kind, added_at)
SELECT id, crate_id, library_track_id, kind, added_at FROM crate_entry ORDER BY id;

DROP TABLE crate_entry;
ALTER TABLE crate_entry_new RENAME TO crate_entry;
CREATE INDEX crate_entry_by_library_track ON crate_entry (library_track_id);

CREATE TRIGGER crate_entry_fits_its_crate_insert
BEFORE INSERT ON crate_entry
WHEN (SELECT CASE kind
          WHEN 'static' THEN new.kind = 'member'
          WHEN 'smart' THEN new.kind IN ('always_include', 'never_include')
          ELSE 0 END
      FROM crate WHERE id = new.crate_id) IS NOT 1
BEGIN
    SELECT RAISE(ABORT, 'a folder holds no tracks, a static crate holds members, and a smart crate holds always-include and never-include tracks');
END;

CREATE TRIGGER crate_entry_fits_its_crate_update
BEFORE UPDATE OF kind, crate_id ON crate_entry
WHEN (SELECT CASE kind
          WHEN 'static' THEN new.kind = 'member'
          WHEN 'smart' THEN new.kind IN ('always_include', 'never_include')
          ELSE 0 END
      FROM crate WHERE id = new.crate_id) IS NOT 1
BEGIN
    SELECT RAISE(ABORT, 'a folder holds no tracks, a static crate holds members, and a smart crate holds always-include and never-include tracks');
END;

-- A Library track joins a crate, leaves it, or moves to another.
CREATE TRIGGER search_crate_entry_added
AFTER INSERT ON crate_entry
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.id = new.library_track_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

CREATE TRIGGER search_crate_entry_removed
AFTER DELETE ON crate_entry
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.id = old.library_track_id
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

CREATE TRIGGER search_crate_entry_changed
AFTER UPDATE OF library_track_id, kind ON crate_entry
WHEN old.library_track_id IS NOT new.library_track_id OR old.kind IS NOT new.kind
BEGIN
    INSERT INTO search_due (recording_id)
    SELECT lt.recording_id FROM library_track lt
    WHERE lt.id IN (old.library_track_id, new.library_track_id)
      AND NOT EXISTS (SELECT 1 FROM search_due d WHERE d.recording_id = lt.recording_id);
END;

-- The count SQLite keeps for the table: past the highest id it holds and
-- past every id of it the operation log names (an operation waiting for an
-- undo may name a row deleted before this migration ran).
INSERT INTO sqlite_sequence (name, seq)
SELECT 'crate_entry', 0
WHERE 'crate_entry' NOT IN (SELECT name FROM sqlite_sequence);

UPDATE sqlite_sequence
SET seq = max(seq,
              (SELECT ifnull(max(id), 0) FROM crate_entry),
              (SELECT ifnull(max(entity_id), 0) FROM change WHERE entity = 'crate_entry'))
WHERE name = 'crate_entry';

-- 2. `sent_playlist.replaced`: for a playlist, the Library tracks whose
--    membership the send that last wrote it changed in rekordbox (a JSON
--    array of ids): each was in rekordbox's playlist of that name, in the
--    newest read when the send was recorded, and the send left it out, or
--    the other way round. NULL when that isn't known (rekordbox's read
--    had no such playlist, a folder, a send before this migration) or once
--    a read showed the send taken up. The merge reads it as the
--    membership the send replaced (1.10, decision table row 7: a send
--    never imported isn't "the DJ changed the playlist back").
ALTER TABLE sent_playlist ADD COLUMN replaced TEXT
    CHECK (replaced IS NULL OR (json_valid(replaced) AND json_type(replaced) = 'array'));
