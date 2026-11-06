-- 0048: wishlists (ROADMAP 2.3, §2): `wishlist`, `wish_item`.
--
-- A wishlist is a list of tracks the user wants, pasted as text or read from
-- a CSV file (Exportify and the like). An item keeps the artist and title as
-- the source gave them.
--
-- No match result is stored. "Owned", "owned, other cut", "you have a
-- rework" and "missing" are worked out when the list is read, from what the
-- app knows by then, like the Inbox labels.
--
-- AUTOINCREMENT on both: every write goes through the operation log, and a
-- deleted wishlist or item must never get its id back after Undo has put a
-- row with that id somewhere else.
--
-- Deleting a wishlist deletes its items; the app does it item by item
-- through the log so that Undo can bring them back. `acquired_file_id` and
-- `acquired_at` are for the auto tick-off (2.5): the Inbox file that
-- answered the wish. A file that goes away leaves the item acquired, with
-- no file.

CREATE TABLE wishlist (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT    NOT NULL CHECK (trim(name) <> ''),
    source     TEXT    NOT NULL CHECK (source IN ('paste', 'csv')),
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE wish_item (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    wishlist_id      INTEGER NOT NULL REFERENCES wishlist (id) ON DELETE CASCADE,
    -- The order the source listed them in; never renumbered.
    position         INTEGER NOT NULL CHECK (position >= 0),
    raw_artist       TEXT    NOT NULL CHECK (trim(raw_artist) <> ''),
    raw_title        TEXT    NOT NULL CHECK (trim(raw_title) <> ''),
    raw_version      TEXT    CHECK (raw_version IS NULL OR trim(raw_version) <> ''),
    raw_isrc         TEXT    CHECK (raw_isrc IS NULL OR trim(raw_isrc) <> ''),
    bought           INTEGER NOT NULL DEFAULT 0 CHECK (bought IN (0, 1)),
    notes            TEXT,
    acquired_file_id INTEGER REFERENCES file (id) ON DELETE SET NULL,
    acquired_at      TEXT,
    created_at       TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (wishlist_id, position)
) STRICT;

CREATE INDEX wish_item_acquired_file ON wish_item (acquired_file_id)
    WHERE acquired_file_id IS NOT NULL;
