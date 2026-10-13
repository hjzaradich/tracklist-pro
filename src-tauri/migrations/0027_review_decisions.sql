-- 0027: what the user decided in Review about a Library track's file
-- (1bE-5, 1bE-7; ROADMAP 1.12), and the rekordbox entries a switch of the
-- linked file left behind (ROADMAP 1.9 rule 7).

-- A decision about a Library track's linked file, kept by what the files
-- hold, not by file or track id (as `grouping_decision` is, migration
-- 0022): a key is a file's `audio_hash`, or its `blake3` when it has none.
-- So a decision survives a rescan, a retag, a move and a music folder
-- removed and added again. A file that is encoded again holds other audio:
-- it has another key, and the decision doesn't cover it. (A `blake3`
-- doesn't survive a tag rewrite: a decision resting on a file with no
-- `audio_hash` is lost with one.)
--
--   dismissed_better: "Better file available" was dismissed for this linked
--                     file and this one better file (`other_key`). Another
--                     better file is another row, so a new item.
--   keep_anyway:      the linked file is flagged, and the user keeps it. The
--                     track leaves "Quality problems"; the ranking still
--                     treats the file as flagged (the verdict isn't changed).
--   want_better:      the user wants this track in better quality. Nothing
--                     reads it yet: the wishlist does, in Phase 2.
--
-- A rowid table with a unique key, so the operation log can record it:
-- every row is made by a user action and taken away by its Undo.
CREATE TABLE review_decision (
    id         INTEGER PRIMARY KEY,
    kind       TEXT    NOT NULL
        CHECK (kind IN ('dismissed_better', 'keep_anyway', 'want_better')),
    -- What the Library track's linked file held when the user decided.
    linked_key BLOB    NOT NULL CHECK (length(linked_key) > 0),
    -- What the dismissed better file held; empty for the other kinds.
    other_key  BLOB    NOT NULL DEFAULT x'',
    decided_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Also the index every lookup uses: a kind, then the linked file's key.
    UNIQUE (kind, linked_key, other_key),
    CHECK ((kind = 'dismissed_better') = (length(other_key) > 0))
) STRICT;

-- A rekordbox entry that a switch of a Library track's linked file left
-- behind: the track was sent as this entry while it was linked to the file
-- that is gone, and linked to the copy it goes to rekordbox as another
-- track. An XML import can't remove the old entry, so it is listed with the
-- tracks to remove in rekordbox, while a read still holds an entry at this
-- `Location` and no Library track is sent as it (a match confirmed later
-- makes the record moot; it is then not listed).
--
-- Made in the switch's own operation, so Undo takes it away. A send never
-- reads it. `recording_id` is not a foreign key on purpose: rekordbox still
-- holds the entry after the track is removed from the Library, or merged
-- into another by grouping, so the record stays whatever becomes of the
-- track (the list names the entry by what rekordbox calls it).
CREATE TABLE replaced_entry (
    id           INTEGER PRIMARY KEY,
    -- The track (`recording`) whose Library track was switched.
    recording_id INTEGER NOT NULL,
    -- The entry's `Location` exactly as rekordbox wrote it (shown as a
    -- path), and decoded as tracks are matched on it (`rekordbox_track`).
    location     TEXT    NOT NULL CHECK (location <> ''),
    location_key TEXT    NOT NULL UNIQUE CHECK (location_key <> ''),
    replaced_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
