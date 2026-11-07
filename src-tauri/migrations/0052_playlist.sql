-- 0052: playlists (ROADMAP 3.5, §2): `playlist`, `playlist_entry`, and
-- `rekordbox_playlist_entry`, rekordbox's playlists in their order as of the
-- last read.
--
-- A playlist is ordered, unlike a crate: each entry has a `position`, dense
-- from 0, rewritten when the user reorders. A track may appear in a
-- playlist more than once. A send writes each playlist under the app's
-- `Playlists` folder in position order.
--
-- `name_key` is the name as the XML writer compares siblings (NFC, letter
-- case and trailing whitespace ignored), worked out by the app: unique, so
-- no two playlists can make a send refuse the folder, and an Undo that
-- would bring back a name another playlist has now is refused.
--
-- `origin` 'rekordbox': copied once from a rekordbox playlist, whose path
-- (a JSON array of names from below ROOT) is kept for display only. A later
-- read never changes it. `target_energy` is the shape the user draws for
-- the timeline (P2), JSON; NULL until drawn.
--
-- AUTOINCREMENT on both: every write goes through the operation log, and a
-- row Undo deleted must never have its id taken by another. No unique index
-- on (playlist_id, position): a reorder and its Undo rewrite positions one
-- row at a time.

CREATE TABLE playlist (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT    NOT NULL CHECK (trim(name) <> '' AND name = trim(name)),
    name_key      TEXT    NOT NULL UNIQUE CHECK (name_key <> ''),
    origin        TEXT    NOT NULL DEFAULT 'app' CHECK (origin IN ('app', 'rekordbox')),
    source_path   TEXT
        CHECK (source_path IS NULL
               OR (json_valid(source_path) AND json_type(source_path) = 'array')),
    target_energy TEXT    CHECK (target_energy IS NULL OR json_valid(target_energy)),
    created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A copy of a rekordbox playlist says which; an app playlist has none.
    CHECK ((origin = 'rekordbox') = (source_path IS NOT NULL))
) STRICT;

CREATE TABLE playlist_entry (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id      INTEGER NOT NULL REFERENCES playlist (id) ON DELETE CASCADE,
    -- Removing a Library track deletes its entries first, in the same
    -- operation, so Undo puts them back.
    library_track_id INTEGER NOT NULL REFERENCES library_track (id) ON DELETE RESTRICT,
    position         INTEGER NOT NULL CHECK (position >= 0)
) STRICT;

CREATE INDEX playlist_entry_order ON playlist_entry (playlist_id, position);
CREATE INDEX playlist_entry_by_track ON playlist_entry (library_track_id);

-- rekordbox's playlists as of the last read, every entry in rekordbox's
-- order: a read-only snapshot like `rekordbox_track`, replaced by every
-- read (XML and database). `playlist_index` is the playlist's place in the
-- tree, depth first, so two playlists with the same path stay apart.
-- `track_id` is the entry's rekordbox TrackID, valid within the read (the
-- `rekordbox_track` row it names), NULL when the entry names no track that
-- was stored.
CREATE TABLE rekordbox_playlist_entry (
    id             INTEGER PRIMARY KEY,
    playlist_index INTEGER NOT NULL CHECK (playlist_index >= 0),
    path           TEXT    NOT NULL
        CHECK (json_valid(path) AND json_type(path) = 'array' AND json_array_length(path) > 0),
    position       INTEGER NOT NULL CHECK (position >= 0),
    track_id       INTEGER,
    read_at        TEXT    NOT NULL,
    UNIQUE (playlist_index, position)
) STRICT;

CREATE INDEX rekordbox_playlist_entry_by_path ON rekordbox_playlist_entry (path);

CREATE TRIGGER rekordbox_playlist_entry_is_read_only
BEFORE UPDATE ON rekordbox_playlist_entry
BEGIN
    SELECT RAISE(ABORT, 'the rekordbox snapshot is read-only; replace it with a fresh read');
END;
