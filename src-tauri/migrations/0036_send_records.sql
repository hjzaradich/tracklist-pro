-- 0036: what each send replaced, and which sent playlists each track went
-- into (ROADMAP §2 `sync_base` and `sent_entry`, 1.10; 1cB-19).
--
-- `sync_base.replaced`: the value rekordbox showed for the field when the
-- send was recorded, which the base sent over it replaced: from rekordbox's
-- entry at the `Location` sent, in the newest read. NULL when there's no
-- such record: rekordbox had no trusted entry for the track, the base is
-- the value rekordbox showed (by the field's kind: nothing was replaced),
-- or a later read showed the send taken up and the record was forgotten.
-- Empty when rekordbox's entry had no such attribute. The conflict engine
-- uses it to tell a send never imported from a change back in rekordbox
-- (1.10, decision table row 7). Rows from before this migration have none.
ALTER TABLE sync_base ADD COLUMN replaced TEXT;

-- `send`: one row per send recorded, numbering them. Its ids only grow
-- (AUTOINCREMENT), so a send's number is never another's, whatever the
-- clock says. Not in the operation log; nothing deletes a row.
CREATE TABLE send (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    sent_at TEXT    NOT NULL
) STRICT;

-- `sent_playlist.send_id`: the last send that wrote the path. NULL for a
-- path last written before this migration.
ALTER TABLE sent_playlist ADD COLUMN send_id INTEGER;

-- `sent_entry`: per Library track, each sent playlist (under `Crates` or
-- `Playlists`) the last send that wrote that playlist put it in, so a
-- track's playlist membership has a base. `path` and `path_key` are as in
-- `sent_playlist`: the names from the top-level folder down, as written,
-- and the same names normalized as the writer compares siblings. One row
-- per track and playlist, however often the playlist names the track.
-- `send_id` is the send that put it there: a row counts only while it's
-- the send that last wrote its playlist (`sent_playlist.send_id`), so a row
-- an Undo put back after a later send rewrote the playlist doesn't. The
-- rows of a playlist go when its `sent_playlist` row goes.
--
-- A rowid table with a unique (library_track_id, path_key), so the
-- operation log can record it: removing a Library track, or merging one
-- away, deletes its rows in the same operation, and Undo puts them back.
-- A send isn't an operation and writes rows here outside the log, so ids
-- are never used twice (AUTOINCREMENT), as for `sync_base`.
CREATE TABLE sent_entry (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    library_track_id INTEGER NOT NULL REFERENCES library_track (id) ON DELETE RESTRICT,
    path             TEXT    NOT NULL
        CHECK (json_valid(path) AND json_type(path) = 'array' AND json_array_length(path) >= 2),
    path_key         TEXT    NOT NULL
        CHECK (json_valid(path_key) AND json_type(path_key) = 'array'
               AND json_array_length(path_key) = json_array_length(path)),
    send_id          INTEGER NOT NULL,
    sent_at          TEXT    NOT NULL,
    UNIQUE (library_track_id, path_key)
) STRICT;
