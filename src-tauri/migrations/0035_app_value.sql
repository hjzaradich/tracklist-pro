-- 0035: the app's own value per field of a Library track (ROADMAP §2
-- `app_value`, 1.10; 1cB-18).
--
-- A row is a field the app means to change (1.9 rule 1): the user's edit,
-- as typed (compared by the field's kind only when it's used, never
-- normalized here), and the value rekordbox showed for the field when the
-- edit was made, which the edit replaced. `replaced` is NULL when that
-- isn't known (rekordbox had no trusted entry for the track) and empty when
-- rekordbox's entry had no such attribute. A field without a row follows
-- rekordbox.
--
-- `field` is the rekordbox XML attribute name, as in `sync_base` and
-- `conflict`. Never one of rekordbox's analysis fields (the app doesn't edit
-- them, 1.9 rule 3, 1.10), never `TrackID` (rekordbox reassigns it on
-- import) and never `Location` (where the linked file is, not an edit).
--
-- A rowid table with a unique (library_track_id, field), so the operation
-- log can record it: each set and clear is one undoable operation, and
-- removing a Library track deletes its rows in the same operation. Its ids
-- are never used twice (AUTOINCREMENT), as for `sync_base` and `conflict`:
-- the send and the merge before it aren't operations, and they are to
-- change rows here outside the log (an app value rekordbox's took over goes,
-- 1cB-19 and 1cB-3), so a new row must never take the id of one an
-- operation deleted.
CREATE TABLE app_value (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    library_track_id INTEGER NOT NULL REFERENCES library_track (id) ON DELETE RESTRICT,
    field            TEXT    NOT NULL
        CHECK (field <> ''
               AND field NOT IN ('AverageBpm', 'Tonality', 'TEMPO', 'POSITION_MARK',
                                 'TrackID', 'Location')),
    -- The app's value; empty when the app wants the field empty.
    value            TEXT    NOT NULL,
    -- What rekordbox showed that `value` replaced; NULL when not known.
    replaced         TEXT,
    UNIQUE (library_track_id, field)
) STRICT;
