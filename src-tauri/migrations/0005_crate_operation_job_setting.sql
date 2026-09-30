-- 0005: crates, the operation log, background jobs, settings and the
-- network opt-ins (ROADMAP 0.1, 1.14, §2).

-- A crate, or a folder of crates. Exported to rekordbox under `Crates` with
-- the same tree (1.9 rule 6, 1.14).
CREATE TABLE crate (
    id         INTEGER PRIMARY KEY,
    -- The folder it's in; NULL at the top of the tree.
    parent_id  INTEGER REFERENCES crate (id) ON DELETE RESTRICT,
    -- folder: holds crates and folders, never tracks. static: hand-made.
    -- smart: rule-based (Phase 3, 3.3).
    kind       TEXT    NOT NULL CHECK (kind IN ('folder', 'static', 'smart')),
    name       TEXT    NOT NULL CHECK (name <> ''),
    -- A smart crate's rules as JSON; only smart crates have them.
    rules      TEXT    CHECK (rules IS NULL OR json_valid(rules)),
    notes      TEXT,
    -- Order among its siblings.
    position   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((kind = 'smart') = (rules IS NOT NULL))
) STRICT;

-- rekordbox replaces a same-name playlist in the same folder on import
-- (§5.2), so two siblings with one name would overwrite each other.
CREATE UNIQUE INDEX crate_names_unique_among_siblings ON crate (ifnull(parent_id, 0), name);
CREATE INDEX crate_by_parent ON crate (parent_id);

-- Every (crate, ancestor) pair, for keeping the tree a tree.
CREATE VIEW crate_ancestor (crate_id, ancestor_id) AS
WITH RECURSIVE up (crate_id, ancestor_id) AS (
    SELECT id, parent_id FROM crate WHERE parent_id IS NOT NULL
    UNION
    SELECT up.crate_id, c.parent_id FROM up JOIN crate c ON c.id = up.ancestor_id
    WHERE c.parent_id IS NOT NULL
)
SELECT crate_id, ancestor_id FROM up;

CREATE TRIGGER crate_parent_is_a_folder_insert
BEFORE INSERT ON crate
WHEN new.parent_id IS NOT NULL
 AND (SELECT kind FROM crate WHERE id = new.parent_id) IS NOT 'folder'
BEGIN
    SELECT RAISE(ABORT, 'a crate can only be inside a folder');
END;

CREATE TRIGGER crate_parent_is_a_folder_update
BEFORE UPDATE OF parent_id ON crate
WHEN new.parent_id IS NOT NULL
 AND (SELECT kind FROM crate WHERE id = new.parent_id) IS NOT 'folder'
BEGIN
    SELECT RAISE(ABORT, 'a crate can only be inside a folder');
END;

CREATE TRIGGER crate_folder_not_inside_itself
BEFORE UPDATE OF parent_id ON crate
WHEN new.parent_id = new.id
  OR EXISTS (SELECT 1 FROM crate_ancestor
             WHERE crate_id = new.parent_id AND ancestor_id = new.id)
BEGIN
    SELECT RAISE(ABORT, 'a folder cannot be moved inside itself');
END;

-- A folder, a static crate and a smart crate hold different things, so a
-- crate stays the kind it was made as.
CREATE TRIGGER crate_kind_is_fixed
BEFORE UPDATE OF kind ON crate
WHEN new.kind <> old.kind
BEGIN
    SELECT RAISE(ABORT, 'a crate cannot change kind');
END;

-- A Library track in a crate. Static crates hold members; smart crates
-- hold their always-include and never-include lists (3.3); folders hold
-- no tracks.
CREATE TABLE crate_entry (
    crate_id         INTEGER NOT NULL REFERENCES crate (id) ON DELETE CASCADE,
    library_track_id INTEGER NOT NULL REFERENCES library_track (id) ON DELETE RESTRICT,
    kind             TEXT    NOT NULL DEFAULT 'member'
        CHECK (kind IN ('member', 'always_include', 'never_include')),
    added_at         TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A track is in a crate once.
    PRIMARY KEY (crate_id, library_track_id)
) STRICT, WITHOUT ROWID;

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

-- The operation log: one row per user action, holding every field change
-- it made, so it can be undone (0.2, ported from musicmanager).
CREATE TABLE operation (
    id         INTEGER PRIMARY KEY,
    kind       TEXT    NOT NULL CHECK (kind <> ''),
    -- What the UI needs to describe it, as JSON. No text is stored: the
    -- description is built from the locale file (i18next).
    details    TEXT    NOT NULL DEFAULT '{}'
        CHECK (json_valid(details) AND json_type(details) = 'object'),
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    undone_at  TEXT
) STRICT;

-- One field of one row, before and after an operation.
CREATE TABLE change (
    id           INTEGER PRIMARY KEY,
    operation_id INTEGER NOT NULL REFERENCES operation (id) ON DELETE CASCADE,
    -- The table and row changed, e.g. `library_track` 12.
    entity       TEXT    NOT NULL CHECK (entity <> ''),
    entity_id    INTEGER NOT NULL,
    -- set: a field of an existing row changed; insert: the row was created
    -- with this value; delete: the row was removed, and had this value.
    -- NULL in before or after is then always a real NULL value, never a
    -- stand-in for "no row".
    action       TEXT    NOT NULL CHECK (action IN ('set', 'insert', 'delete')),
    field        TEXT    NOT NULL CHECK (field <> ''),
    before       TEXT,
    after        TEXT,
    CHECK (CASE action
        WHEN 'set' THEN before IS NOT after
        WHEN 'insert' THEN before IS NULL
        WHEN 'delete' THEN after IS NULL
    END)
) STRICT;

CREATE INDEX change_by_operation ON change (operation_id);

-- Background work (Activity in the UI), persisted so it survives a crash
-- (0.1).
CREATE TABLE job (
    id          INTEGER PRIMARY KEY,
    -- scan, hash, fingerprint, analyze, embed, convert, export, …
    kind        TEXT    NOT NULL CHECK (kind <> ''),
    -- What it works on, as JSON.
    target      TEXT    CHECK (target IS NULL OR json_valid(target)),
    priority    INTEGER NOT NULL DEFAULT 0,
    status      TEXT    NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'done', 'failed', 'cancelled')),
    progress    REAL    CHECK (progress BETWEEN 0 AND 1),
    error       TEXT,
    attempts    INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    started_at  TEXT,
    finished_at TEXT,
    -- A job has finished exactly when it's done, failed or cancelled.
    CHECK ((status IN ('done', 'failed', 'cancelled')) = (finished_at IS NOT NULL)),
    -- A failed job says why.
    CHECK (status <> 'failed' OR error IS NOT NULL)
) STRICT;

-- For picking the next job: queued, highest priority, oldest first.
CREATE INDEX job_queue ON job (status, priority DESC, id);

-- App settings, each a JSON value.
CREATE TABLE setting (
    key        TEXT NOT NULL PRIMARY KEY CHECK (key <> ''),
    value      TEXT NOT NULL CHECK (json_valid(value)),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT, WITHOUT ROWID;

-- The user's opt-in for each online service; the network gate checks it
-- before every request (0.1). Offline by default: a service with no row,
-- or a row left at its default, is not opted in. Nothing here opts in.
CREATE TABLE service_optin (
    -- e.g. musicbrainz, update_check, model_download.
    service    TEXT    NOT NULL PRIMARY KEY
        CHECK (service <> '' AND service NOT GLOB '*[^a-z0-9_]*'),
    enabled    INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    -- When the user opted in.
    enabled_at TEXT,
    CHECK ((enabled = 1) = (enabled_at IS NOT NULL))
) STRICT, WITHOUT ROWID;
