-- 0047: the Inbox's intake (ROADMAP 2.1, §2): `inbox_batch`, `inbox_item`.
--
-- A batch is the way files arrived in an Inbox folder (a folder's worth,
-- loose files, or what was already there when the folder was added). An
-- item is one file in the Inbox and what the user decided about it.
--
-- No labels are stored. "New to you", "Better file" and the rest are worked
-- out when the Inbox is read, from what the app knows by then.
--
-- AUTOINCREMENT on both: intake writes these rows outside the operation
-- log, so an id must never come back after its row is gone.
--
-- Derived state, like `file_stage`: removing a music folder deletes its
-- files, and with them their items; and then its batches. A RESTRICT here
-- would make every Inbox folder "in use".

CREATE TABLE inbox_batch (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    music_folder_id INTEGER NOT NULL REFERENCES music_folder (id) ON DELETE CASCADE,
    kind            TEXT    NOT NULL CHECK (kind IN ('loose', 'folder', 'already_there')),
    -- The subfolder (relative to the music folder) a 'folder' batch came
    -- in: exactly then.
    label           TEXT    CHECK (label IS NULL OR label <> ''),
    arrived_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((kind = 'folder') = (label IS NOT NULL))
) STRICT;

CREATE INDEX inbox_batch_music_folder ON inbox_batch (music_folder_id);

CREATE TABLE inbox_item (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The `file` row (file.id). A file is in the Inbox once.
    file_id    INTEGER NOT NULL UNIQUE REFERENCES file (id) ON DELETE CASCADE,
    batch_id   INTEGER NOT NULL REFERENCES inbox_batch (id) ON DELETE CASCADE,
    status     TEXT    NOT NULL DEFAULT 'new'
               CHECK (status IN ('new', 'not_now', 'hidden', 'added')),
    -- When it counted as arrived: it was there, and held still.
    arrived_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- When the user decided; nothing while it is new.
    decided_at TEXT,
    CHECK (status <> 'new' OR decided_at IS NULL)
) STRICT;

CREATE INDEX inbox_item_batch ON inbox_item (batch_id);
CREATE INDEX inbox_item_status ON inbox_item (status);
