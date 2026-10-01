-- 0015: the playlists and folders a send has written (ROADMAP 1.9 rule 6).
--
-- rekordbox can't be told to delete a playlist through XML, so after a
-- crate is renamed or deleted in the app its old rekordbox playlist stays
-- until the user deletes it by hand. The app lists only what it sent
-- itself: a playlist or folder the user made in rekordbox is never listed,
-- even inside the `Crates` and `Playlists` folders.
--
-- One row per path a send wrote, kept across sends (written in the same
-- transaction as the send's `sync_base` rows). `path` holds the folder and
-- playlist names from the top-level folder down, as last written;
-- `path_key` the same names normalized the way the writer compares sibling
-- names (NFC, letter case and trailing whitespace ignored), which is what a
-- path is looked up by. `seen` is set once a rekordbox read has shown the
-- path there; a read that then lacks it deletes the row (the user deleted
-- it). A row never seen stays: the user may not have imported that send yet.
-- Not undoable and not in the operation log, like the send itself.
CREATE TABLE sent_playlist (
    id       INTEGER PRIMARY KEY,
    path     TEXT    NOT NULL
        CHECK (json_valid(path) AND json_type(path) = 'array' AND json_array_length(path) >= 2),
    path_key TEXT    NOT NULL
        CHECK (json_valid(path_key) AND json_type(path_key) = 'array'
               AND json_array_length(path_key) = json_array_length(path)),
    kind     TEXT    NOT NULL CHECK (kind IN ('folder', 'playlist')),
    seen     INTEGER NOT NULL DEFAULT 0 CHECK (seen IN (0, 1)),
    sent_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (path_key, kind)
) STRICT;
