-- 0013: keeping the Library in step with the files,
-- and remembering what the user removed (ROADMAP 1.3, 1.9 rule 7, §2).

-- A linked Library track's `source_status` follows its file's `present`,
-- whichever code changes either one (a walk, a rescan, a relink), so it's
-- never computed only when the list is read. A file on an unplugged drive
-- stays present, so its track isn't flagged.
CREATE TRIGGER library_track_status_follows_file
AFTER UPDATE OF present ON file
BEGIN
    UPDATE library_track
    SET source_status = CASE WHEN new.present = 1 THEN 'ok' ELSE 'missing' END
    WHERE kind = 'linked' AND linked_file_id = new.id
      AND source_status <> CASE WHEN new.present = 1 THEN 'ok' ELSE 'missing' END;
END;

-- A linked track pointed at another file (or added) takes that file's state.
CREATE TRIGGER library_track_status_follows_new_link_update
AFTER UPDATE OF linked_file_id ON library_track
WHEN new.kind = 'linked' AND EXISTS (SELECT 1 FROM file WHERE id = new.linked_file_id)
BEGIN
    UPDATE library_track
    SET source_status = coalesce((SELECT CASE WHEN present = 1 THEN 'ok' ELSE 'missing' END
                                  FROM file WHERE id = new.linked_file_id), source_status)
    WHERE id = new.id;
END;

CREATE TRIGGER library_track_status_follows_new_link_insert
AFTER INSERT ON library_track
WHEN new.kind = 'linked' AND EXISTS (SELECT 1 FROM file WHERE id = new.linked_file_id)
BEGIN
    UPDATE library_track
    SET source_status = coalesce((SELECT CASE WHEN present = 1 THEN 'ok' ELSE 'missing' END
                                  FROM file WHERE id = new.linked_file_id), source_status)
    WHERE id = new.id;
END;

-- Bring the rows that exist up to date.
UPDATE library_track
SET source_status = (SELECT CASE WHEN f.present = 1 THEN 'ok' ELSE 'missing' END
                     FROM file f WHERE f.id = library_track.linked_file_id)
WHERE kind = 'linked' AND linked_file_id IS NOT NULL;

-- A track the user removed from the Library in the app, and hasn't added
-- back. It keeps rekordbox's tracks the user removed out of the offer to
-- add them (1.3), and tells a send which tracks the user has to remove in
-- rekordbox by hand, because XML can't remove them (1.9 rule 7).
-- Made in the same operation as the removal, so undo takes it away again;
-- adding the track back deletes it in that operation.
CREATE TABLE library_removal (
    id                 INTEGER PRIMARY KEY,
    -- The track that was removed. One record per track.
    recording_id       INTEGER NOT NULL UNIQUE REFERENCES recording (id) ON DELETE RESTRICT,
    removed_at         TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Where the Library track was last sent to rekordbox, copied from it:
    -- NULL if it never was, so rekordbox has nothing to remove.
    last_sent_location TEXT,
    last_exported_at   TEXT
) STRICT;

-- The operation log records rowid tables only (ops), and removing a Library
-- track has to delete its crate entries and sync bases undoably. So these
-- two tables are rebuilt as rowid tables: the same columns, checks and
-- foreign keys, a row id for the log, and the old primary key kept as a
-- UNIQUE key, so upserts on those columns work as before.
CREATE TABLE crate_entry_new (
    id               INTEGER PRIMARY KEY,
    crate_id         INTEGER NOT NULL REFERENCES crate (id) ON DELETE CASCADE,
    library_track_id INTEGER NOT NULL REFERENCES library_track (id) ON DELETE RESTRICT,
    kind             TEXT    NOT NULL DEFAULT 'member'
        CHECK (kind IN ('member', 'always_include', 'never_include')),
    added_at         TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A track is in a crate once.
    UNIQUE (crate_id, library_track_id)
) STRICT;

INSERT INTO crate_entry_new (crate_id, library_track_id, kind, added_at)
SELECT crate_id, library_track_id, kind, added_at FROM crate_entry
ORDER BY crate_id, library_track_id;

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

CREATE TABLE sync_base_new (
    id               INTEGER PRIMARY KEY,
    library_track_id INTEGER NOT NULL REFERENCES library_track (id) ON DELETE RESTRICT,
    -- The field as sent, e.g. an XML attribute name such as `Rating`.
    field            TEXT    NOT NULL CHECK (field <> ''),
    value            TEXT    NOT NULL,
    synced_at        TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- One base value per track and field.
    UNIQUE (library_track_id, field)
) STRICT;

INSERT INTO sync_base_new (library_track_id, field, value, synced_at)
SELECT library_track_id, field, value, synced_at FROM sync_base
ORDER BY library_track_id, field;

DROP TABLE sync_base;
ALTER TABLE sync_base_new RENAME TO sync_base;
