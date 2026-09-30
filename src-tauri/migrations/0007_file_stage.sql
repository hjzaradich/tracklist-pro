-- 0007: what each later scan stage last did to each file (ROADMAP 1.1
-- stages 2 and 3), so a rescan only redoes files that changed.
--
-- One row per file per stage, recording the file's size and mtime as the
-- stage saw them and the version of the stage's code. A file is due for a
-- stage again when either differs from its `file` row (scan_state.rs).
--
-- Derived state: deleting a file row (as removing its music folder does)
-- deletes its rows here too. A RESTRICT here would make every scanned
-- folder "in use".
CREATE TABLE file_stage (
    -- The `file` row (file.id), not the volume's file id (file.file_id).
    file_id INTEGER NOT NULL REFERENCES file (id) ON DELETE CASCADE,
    stage   TEXT    NOT NULL CHECK (stage IN ('read', 'hash', 'fingerprint')),
    -- The stage's own version number; bumping it makes every file due.
    version INTEGER NOT NULL CHECK (version >= 1),
    -- file.size and file.mtime when the stage ran. NULL where the file row
    -- had none.
    size    INTEGER CHECK (size >= 0),
    mtime   INTEGER,
    status  TEXT    NOT NULL CHECK (status IN ('done', 'failed', 'skipped')),
    -- A short code saying why the stage failed or skipped the file, e.g.
    -- 'online_only'. Never UI text.
    reason  TEXT    CHECK (reason IS NULL OR reason <> ''),
    done_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A reason exactly when the stage didn't finish the file.
    CHECK ((status = 'done') = (reason IS NULL)),
    PRIMARY KEY (file_id, stage)
) STRICT;
