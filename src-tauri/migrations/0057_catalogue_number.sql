-- 0057: the app's own catalogue number per Library track (ROADMAP 2.2,
-- owner decisions 2026-11-08; Run 6A, S1).
--
-- rekordbox's XML has no attribute for a catalogue number, so it can't be
-- an app value (`app_value` holds rekordbox `TRACK` attributes, all sent):
-- the app keeps it here and never sends it. It waits for file writing
-- (2.6). A Library track has one at most; a track without a row has none.
--
-- Written only through the operation log ("Use" on a look-up,
-- `identify::use_on`), so a rowid STRICT table, and its ids are never used
-- twice (AUTOINCREMENT), as for `app_value`. Removing a Library track and
-- merging two delete its row in their own operation, so RESTRICT.
CREATE TABLE catalogue_number (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    library_track_id INTEGER NOT NULL UNIQUE
        REFERENCES library_track (id) ON DELETE RESTRICT,
    -- As the user chose it, outer blanks dropped; never empty.
    value            TEXT    NOT NULL CHECK (value <> '' AND value = trim(value))
) STRICT;
